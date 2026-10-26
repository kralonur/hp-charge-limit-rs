//! `hp-charge-limit-rs notify`: runs as the desktop user (no root) and turns
//! the daemon's system-bus signals into freedesktop notifications, which
//! KDE, GNOME and most other desktops display.

use crate::events::{Event, INTERFACE, NAME, PATH};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::names::BusName;
use zbus::zvariant::Value;
use zbus::{MatchRule, Message, message};

const APP_NAME: &str = "hp-charge-limit-rs";
const ICON: &str = "battery";
const SERVER: &str = "org.freedesktop.Notifications";
const SERVER_PATH: &str = "/org/freedesktop/Notifications";
const URGENCY_NORMAL: u8 = 1;
const URGENCY_CRITICAL: u8 = 2;
/// Let the notification server pick how long it stays.
const DEFAULT_TIMEOUT: i32 = -1;

struct Toast {
    summary: &'static str,
    body: String,
    urgency: u8,
}

/// Notification bodies may be rendered as markup.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn describe(event: Event) -> Toast {
    let normal = |summary, body: &str| Toast { summary, body: body.into(), urgency: URGENCY_NORMAL };
    match event {
        Event::LimitReached { capacity } => {
            normal("Charge limit reached", &format!("Battery at {capacity}%. Charging is paused."))
        }
        Event::ChargingResumed { capacity } => normal("Charging resumed", &format!("Battery fell to {capacity}%.")),
        Event::BypassChanged { enabled: true } => {
            normal("Charge limit paused", "Bypass is on: the battery may charge fully.")
        }
        Event::BypassChanged { enabled: false } => {
            normal("Charge limit resumed", "Bypass is off: the charge band applies again.")
        }
        Event::WriteFailed { mode, error } => Toast {
            summary: "Charge control failed",
            body: escape(&format!("Could not set {mode}: {error}")),
            urgency: URGENCY_CRITICAL,
        },
    }
}

/// The bus applies the match rule's well-known sender to broadcasts only;
/// a signal sent directly to us passes it from anyone, so require that the
/// sender owns `NAME`, which only root may (see the bus policy).
fn from_daemon(dbus: &DBusProxy, name: &BusName, msg: &Message) -> bool {
    let header = msg.header();
    let Some(sender) = header.sender() else { return false };
    dbus.get_name_owner(name.clone()).is_ok_and(|owner| *owner == *sender)
}

/// Shows `toast` in place of the previous one (`replaces`, 0 for none);
/// returns the id of the notification shown.
fn show(session: &Connection, replaces: u32, toast: &Toast) -> zbus::Result<u32> {
    let hints = HashMap::from([("urgency", Value::from(toast.urgency))]);
    let reply = session.call_method(
        Some(SERVER),
        SERVER_PATH,
        Some(SERVER),
        "Notify",
        &(APP_NAME, replaces, ICON, toast.summary, toast.body.as_str(), Vec::<&str>::new(), hints, DEFAULT_TIMEOUT),
    )?;
    reply.body().deserialize()
}

pub fn run() -> Result<()> {
    let session =
        Connection::session().context("no session bus: run this as your desktop user, inside a login session")?;
    let system = Connection::system().context("cannot reach the system bus")?;
    let dbus = DBusProxy::new(&system)?;
    let name = BusName::try_from(NAME)?;
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(NAME)?
        .path(PATH)?
        .interface(INTERFACE)?
        .build();
    let signals = MessageIterator::for_match_rule(rule, &system, None)?;
    println!("hp-charge-limit-rs notify: waiting for {NAME} signals");

    let mut replaces = 0;
    for msg in signals {
        let msg = msg?;
        if !from_daemon(&dbus, &name, &msg) {
            continue;
        }
        let Some(event) = Event::parse(&msg) else { continue };
        match show(&session, replaces, &describe(event)) {
            Ok(id) => replaces = id,
            Err(e) => eprintln!("notification not shown: {e}"),
        }
    }
    bail!("system bus connection closed")
}
