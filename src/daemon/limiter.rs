//! The charge band:
//!
//! ```text
//!   Charging --(capacity >= stop, run mode set)--> Limiting
//!   Limiting --(capacity <= start, auto set)-----> Charging
//! ```
//!
//! A transition only happens once the firmware accepted the new mode;
//! otherwise the state stays and the next tick retries. `finish` consumes
//! the limiter, so nothing can tick after it.

use crate::firmware::{ChargeControl, Mode};
use crate::power::Reading;
use clap::ValueEnum;
use std::fmt;

#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub stop: u8,
    pub start: u8,
    pub run_mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Starting,
    Charging,
    Limiting,
    Bypassed,
}

pub struct Limiter {
    band: Band,
    state: State,
}

/// Sets `mode` and logs `<label>: cap=..% -> <mode>`; true if it took.
fn set_logged(fw: &mut impl ChargeControl, mode: Mode, label: &str, capacity: u8) -> bool {
    match fw.set(mode) {
        Ok(()) => {
            println!("{label}: cap={capacity}% -> {mode}");
            true
        }
        Err(e) => {
            eprintln!("{label}: cap={capacity}% -> {mode} FAILED: {e:#}");
            false
        }
    }
}

/// Re-set every cycle while in its zone: harmless when already set, and
/// repairs a mode changed behind our back.
fn reassert(fw: &mut impl ChargeControl, mode: Mode) {
    if let Err(e) = fw.set(mode) {
        eprintln!("set {mode}: {e:#}");
    }
}

impl Limiter {
    /// Always sets a mode, replacing whatever was left (e.g. hold from the
    /// last shutdown); above `start` it keeps limiting unless bypassed.
    pub fn start(band: Band, capacity: u8, bypass: bool, fw: &mut impl ChargeControl) -> Self {
        let mut limiter = Limiter { band, state: State::Starting };
        limiter.enter(capacity, bypass, fw);
        limiter
    }

    /// Sets the mode `capacity` and `bypass` call for, ignoring hysteresis:
    /// used at start (retried while it fails) and when bypass begins or ends.
    fn enter(&mut self, capacity: u8, bypass: bool, fw: &mut impl ChargeControl) {
        let (mode, state) = if bypass {
            (Mode::Auto, State::Bypassed)
        } else if capacity <= self.band.start {
            (Mode::Auto, State::Charging)
        } else {
            (self.band.run_mode, State::Limiting)
        };
        let label = match self.state {
            _ if bypass => "bypass",
            State::Bypassed => "resume",
            _ => "start",
        };
        if set_logged(fw, mode, label, capacity) {
            self.state = state;
        }
    }

    pub fn tick(&mut self, reading: Reading, bypass: bool, fw: &mut impl ChargeControl) {
        // on battery nothing charges: leave the mode alone
        if !reading.on_ac {
            return;
        }
        let capacity = reading.capacity;
        let bypassed = self.state == State::Bypassed;
        if bypass && bypassed {
            reassert(fw, Mode::Auto);
            return;
        }
        if bypass || bypassed || self.state == State::Starting {
            self.enter(capacity, bypass, fw);
            return;
        }
        let Band { stop, start, run_mode } = self.band;
        match self.state {
            State::Charging if capacity >= stop => {
                if set_logged(fw, run_mode, "limit", capacity) {
                    self.state = State::Limiting;
                }
            }
            State::Charging if capacity <= start => reassert(fw, Mode::Auto),
            State::Limiting if capacity <= start => {
                if set_logged(fw, Mode::Auto, "release", capacity) {
                    self.state = State::Charging;
                }
            }
            State::Limiting if capacity >= stop => reassert(fw, run_mode),
            State::Starting | State::Charging | State::Limiting | State::Bypassed => {}
        }
    }

    /// Ends the limiter with the mode `policy` picks for `exit`; returns
    /// that mode, or None if the mode was kept.
    pub fn finish(self, exit: Exit, policy: &ExitPolicy, capacity: u8, fw: &mut impl ChargeControl) -> Option<Mode> {
        let label = exit.label();
        let mode = policy.mode(exit, capacity);
        match mode {
            Some(mode) => {
                set_logged(fw, mode, label, capacity);
            }
            None => println!("{label}: cap={capacity}% -> mode kept"),
        }
        mode
    }
}

/// What to do with the firmware mode when the limiter ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum StopAction {
    Auto,
    Inhibit,
    Hold,
    /// Leave whatever mode is set
    Keep,
}

impl StopAction {
    pub fn mode(self) -> Option<Mode> {
        match self {
            StopAction::Auto => Some(Mode::Auto),
            StopAction::Inhibit => Some(Mode::Inhibit),
            StopAction::Hold => Some(Mode::Hold),
            StopAction::Keep => None,
        }
    }
}

impl fmt::Display for StopAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value = self.to_possible_value().expect("no skipped variants");
        f.write_str(value.get_name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The PC is shutting down or rebooting
    Shutdown,
    /// Only the service is stopped
    ServiceStop,
}

impl Exit {
    fn label(self) -> &'static str {
        match self {
            Exit::Shutdown => "shutdown",
            Exit::ServiceStop => "service stop",
        }
    }
}

pub struct ExitPolicy {
    pub off_mode: StopAction,
    /// At or below this %, shutdown uses auto so the battery charges while off
    pub off_min: u8,
    pub stop_mode: StopAction,
}

impl ExitPolicy {
    fn mode(&self, exit: Exit, capacity: u8) -> Option<Mode> {
        match exit {
            Exit::ServiceStop => self.stop_mode.mode(),
            Exit::Shutdown => self
                .off_mode
                .mode()
                .map(|mode| if capacity <= self.off_min { Mode::Auto } else { mode }),
        }
    }
}
