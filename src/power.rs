//! Battery and AC adapter readings from sysfs.

use crate::paths::{self, POWER_SUPPLY};
use std::fmt;
use std::fs;
use std::ops::Sub;
use std::path::PathBuf;

const MICRO_PER_MILLI: i64 = 1_000;
const MICRO_PER_UNIT: f64 = 1_000_000.0;
const ADAPTER_ONLINE: &str = "1";

/// Discover afresh so hotplugged supplies appear on the next read.
/// Prefer BAT1 for compatibility, then use stable alphabetical ordering.
fn supplies() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(paths::at(POWER_SUPPLY)) else { return Vec::new() };
    let mut supplies: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| paths::read_trimmed(&path.join("scope")).as_deref() != Some("Device"))
        .collect();
    supplies.sort_by(|a, b| (!a.ends_with("BAT1"), a).cmp(&(!b.ends_with("BAT1"), b)));
    supplies
}

pub fn battery() -> Option<PathBuf> {
    supplies().into_iter().find(|path| {
        paths::read_trimmed(&path.join("type")).as_deref() == Some("Battery")
            && paths::read_trimmed(&path.join("present")).as_deref() != Some("0")
    })
}

fn battery_attr(name: &str) -> Option<String> {
    paths::read_trimmed(&battery()?.join(name))
}

/// Battery energy in µWh, the unit of sysfs `energy_now`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Energy(i64);

impl Energy {
    pub const fn from_mwh(mwh: i64) -> Self {
        Self(mwh * MICRO_PER_MILLI)
    }
    pub const fn from_uwh(uwh: i64) -> Self {
        Self(uwh)
    }
    pub const fn uwh(self) -> i64 {
        self.0
    }
    pub const fn mwh(self) -> i64 {
        self.0 / MICRO_PER_MILLI
    }
    pub fn wh(self) -> f64 {
        self.0 as f64 / MICRO_PER_UNIT
    }
    pub const fn abs(self) -> Self {
        Self(self.0.abs())
    }
}

impl Sub for Energy {
    type Output = Energy;
    fn sub(self, rhs: Energy) -> Energy {
        Energy(self.0 - rhs.0)
    }
}

impl fmt::Display for Energy {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:.2} Wh", self.wh())
    }
}

/// Battery voltage in µV, the unit of sysfs `voltage_now`.
#[derive(Clone, Copy, Debug)]
pub struct Voltage(i64);

impl fmt::Display for Voltage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:.2} V", self.0 as f64 / MICRO_PER_UNIT)
    }
}

/// What the limiter needs each cycle.
#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub capacity: u8,
    pub on_ac: bool,
}

pub fn capacity() -> Option<u8> {
    battery_attr("capacity")?.parse().ok()
}

pub fn energy() -> Option<Energy> {
    battery_attr("energy_now")?.parse().ok().map(Energy)
}

pub fn voltage() -> Option<Voltage> {
    battery_attr("voltage_now")?.parse().ok().map(Voltage)
}

pub fn status() -> Option<String> {
    battery_attr("status")
}

fn number(name: &str) -> Option<f64> {
    battery_attr(name)?.parse::<i64>().ok().map(|n| n as f64)
}

/// Full capacity relative to design capacity; charge-based batteries are supported too.
pub fn health() -> Option<f64> {
    for (full, design) in [("energy_full", "energy_full_design"), ("charge_full", "charge_full_design")] {
        if let (Some(full), Some(design)) = (number(full), number(design))
            && full >= 0.0 && design > 0.0
        {
            return Some(full / design * 100.0);
        }
    }
    None
}

pub fn cycles() -> Option<u64> {
    battery_attr("cycle_count")?.parse().ok()
}

/// Prefer power_now; otherwise derive watts from current_now and voltage_now.
pub fn power_watts() -> Option<f64> {
    number("power_now").map(|p| p / MICRO_PER_UNIT).or_else(|| {
        Some(number("current_now")? / MICRO_PER_UNIT * number("voltage_now")? / MICRO_PER_UNIT)
    })
}

pub fn manufacturer() -> Option<String> {
    battery_attr("manufacturer")
}

pub fn model() -> Option<String> {
    battery_attr("model_name")
}

pub fn adapters() -> Vec<PathBuf> {
    supplies().into_iter()
        .filter(|path| {
            let kind = paths::read_trimmed(&path.join("type")).unwrap_or_default();
            matches!(kind.as_str(), "Mains" | "Wireless") || kind.starts_with("USB")
        })
        .collect()
}

pub fn on_ac() -> bool {
    adapters().into_iter().any(|path| {
        paths::read_trimmed(&path.join("online")).as_deref() == Some(ADAPTER_ONLINE)
    })
}

pub fn reading() -> Option<Reading> {
    Some(Reading { capacity: capacity()?, on_ac: on_ac() })
}
