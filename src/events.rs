//! D-Bus signals the daemon emits on the system bus, for desktop notifiers.
//!
//! Signals only: no methods, so nothing on the bus can change charging.
//! Emitting is strictly best effort. A missing or broken bus never affects
//! charge control, and only root may own `NAME` (see the bus policy file).

use crate::firmware::Mode;
use std::time::{Duration, Instant};
use zbus::Message;
use zbus::blocking::Connection;

pub const NAME: &str = "io.github.kralonur.HpChargeLimit";
pub const PATH: &str = "/io/github/kralonur/HpChargeLimit";
pub const INTERFACE: &str = "io.github.kralonur.HpChargeLimit";

/// Wait this long before trying to reach the bus again after a failure.
const RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Charging was blocked at the stop threshold
    LimitReached { capacity: u8 },
    /// Charging was allowed again at the start threshold
    ChargingResumed { capacity: u8 },
    /// Bypass took effect (true) or ended (false)
    BypassChanged { enabled: bool },
    /// A firmware write failed; sent once per run of failures
    WriteFailed { mode: Mode, error: String },
}

impl Event {
    fn signal(&self) -> &'static str {
        match self {
            Event::LimitReached { .. } => "LimitReached",
            Event::ChargingResumed { .. } => "ChargingResumed",
            Event::BypassChanged { .. } => "BypassChanged",
            Event::WriteFailed { .. } => "WriteFailed",
        }
    }

    /// The broadcast signal carrying this event.
    fn message(&self) -> zbus::Result<Message> {
        let signal = Message::signal(PATH, INTERFACE, self.signal())?;
        match self {
            Event::LimitReached { capacity } | Event::ChargingResumed { capacity } => signal.build(&(*capacity,)),
            Event::BypassChanged { enabled } => signal.build(&(*enabled,)),
            Event::WriteFailed { mode, error } => signal.build(&(mode.to_string(), error.as_str())),
        }
    }
}

pub struct Emitter {
    bus: Option<Connection>,
    next_try: Instant,
    reported_down: bool,
}

impl Emitter {
    pub fn new() -> Self {
        let mut emitter = Emitter { bus: None, next_try: Instant::now(), reported_down: false };
        emitter.connect();
        emitter
    }

    fn connect(&mut self) {
        let attempt = Connection::system().and_then(|bus| {
            bus.request_name(NAME)?;
            Ok(bus)
        });
        match attempt {
            Ok(bus) => {
                self.bus = Some(bus);
                if self.reported_down {
                    println!("D-Bus: connected, signals enabled");
                }
                self.reported_down = false;
            }
            Err(e) => {
                if !self.reported_down {
                    eprintln!("D-Bus: signals disabled, will retry: {e}");
                    self.reported_down = true;
                }
                self.next_try = Instant::now() + RETRY_AFTER;
            }
        }
    }

    pub fn emit_all(&mut self, events: impl IntoIterator<Item = Event>) {
        for event in events {
            self.emit(&event);
        }
    }

    fn emit(&mut self, event: &Event) {
        if self.bus.is_none() && Instant::now() >= self.next_try {
            self.connect();
        }
        let Some(bus) = &self.bus else { return };
        if let Err(e) = event.message().and_then(|msg| bus.send(&msg)) {
            eprintln!("D-Bus: {} not sent: {e}", event.signal());
            self.bus = None;
            self.next_try = Instant::now() + RETRY_AFTER;
        }
    }
}
