//! `hp-charge-limit-rs daemon`: the limiter loop the service runs.

mod limiter;

pub use limiter::StopAction;

use crate::cli::DaemonOpts;
use crate::events::Emitter;
use crate::firmware::Wmi;
use crate::{bypass, offstate, power, systemd};
use anyhow::{Result, ensure};
use limiter::{Band, Exit, ExitPolicy, Limiter};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

/// Used when capacity can't be read at start or exit: counts as empty, so
/// the battery charges rather than holds.
const UNKNOWN_CAPACITY: u8 = 0;

/// A channel that receives a message on SIGTERM (systemd stop, shutdown) or
/// SIGINT. Registered before it returns, so a signal during startup is kept.
fn termination() -> Result<Receiver<()>> {
    let (tx, rx) = mpsc::channel();
    let mut signals = Signals::new([SIGTERM, SIGINT])?;
    thread::spawn(move || {
        for _ in signals.forever() {
            if tx.send(()).is_err() {
                break;
            }
        }
    });
    Ok(rx)
}

/// Sends the limiter's new events, or drops them with notifications off;
/// taken either way so they never pile up.
fn publish(limiter: &mut Limiter, emitter: Option<&mut Emitter>) {
    let events = limiter.take_events();
    if let Some(emitter) = emitter {
        emitter.emit_all(events);
    }
}

pub fn run(o: &DaemonOpts) -> Result<()> {
    ensure!(o.start < o.stop, "--start ({}) must be below --stop ({})", o.start, o.stop);
    let terminate = termination()?;
    println!(
        "hp-charge-limit-rs: stop={}% start={}% poll={}s run={} off={} (min {}%) stop-mode={} notify={}",
        o.stop, o.start, o.interval, o.run_mode, o.off_mode, o.off_min, o.stop_mode, o.notify
    );
    if let Some(line) = offstate::report() {
        println!("{line}");
    }

    let capacity = || power::capacity().unwrap_or(UNKNOWN_CAPACITY);
    let band = Band { stop: o.stop, start: o.start, run_mode: o.run_mode };
    let interval = Duration::from_secs(o.interval);
    let mut wmi = Wmi;
    let mut emitter = o.notify.then(Emitter::new);
    let mut limiter = Limiter::start(band, capacity(), bypass::enabled(), &mut wmi);
    publish(&mut limiter, emitter.as_mut());
    // wait one interval for a signal; a timeout is the next tick
    while let Err(RecvTimeoutError::Timeout) = terminate.recv_timeout(interval) {
        if let Some(reading) = power::reading() {
            limiter.tick(reading, bypass::enabled(), &mut wmi);
            publish(&mut limiter, emitter.as_mut());
        }
    }

    let exit = if systemd::system_stopping() { Exit::Shutdown } else { Exit::ServiceStop };
    let policy = if bypass::enabled() {
        ExitPolicy { off_mode: StopAction::Auto, off_min: o.off_min, stop_mode: StopAction::Auto }
    } else {
        ExitPolicy { off_mode: o.off_mode, off_min: o.off_min, stop_mode: o.stop_mode }
    };
    let capacity = capacity();
    let left = limiter.finish(exit, &policy, capacity, &mut wmi);
    if exit == Exit::Shutdown {
        let action = left.map_or(StopAction::Keep.to_string(), |mode| mode.to_string());
        offstate::save(capacity, &action);
    }
    Ok(())
}
