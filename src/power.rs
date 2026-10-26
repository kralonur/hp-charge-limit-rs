//! Battery and AC adapter readings from sysfs.

use crate::paths::{self, ADAPTER, BATTERY};
use std::fmt;
use std::ops::Sub;

const MICRO_PER_MILLI: i64 = 1_000;
const MICRO_PER_UNIT: f64 = 1_000_000.0;
const ADAPTER_ONLINE: &str = "1";

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
    paths::attr(BATTERY, "capacity")?.parse().ok()
}

pub fn energy() -> Option<Energy> {
    paths::attr(BATTERY, "energy_now")?.parse().ok().map(Energy)
}

pub fn voltage() -> Option<Voltage> {
    paths::attr(BATTERY, "voltage_now")?.parse().ok().map(Voltage)
}

pub fn status() -> Option<String> {
    paths::attr(BATTERY, "status")
}

fn number(name: &str) -> Option<f64> {
    paths::attr(BATTERY, name)?.parse::<i64>().ok().map(|n| n as f64)
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
    paths::attr(BATTERY, "cycle_count")?.parse().ok()
}

/// Prefer power_now; otherwise derive watts from current_now and voltage_now.
pub fn power_watts() -> Option<f64> {
    number("power_now").map(|p| p / MICRO_PER_UNIT).or_else(|| {
        Some(number("current_now")? / MICRO_PER_UNIT * number("voltage_now")? / MICRO_PER_UNIT)
    })
}

pub fn manufacturer() -> Option<String> {
    paths::attr(BATTERY, "manufacturer")
}

pub fn model() -> Option<String> {
    paths::attr(BATTERY, "model_name")
}

pub fn on_ac() -> bool {
    paths::attr(ADAPTER, "online").as_deref() == Some(ADAPTER_ONLINE)
}

pub fn reading() -> Option<Reading> {
    Some(Reading { capacity: capacity()?, on_ac: on_ac() })
}
