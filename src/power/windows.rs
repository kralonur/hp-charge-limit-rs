//! Windows backend: charge % and AC from GetSystemPowerStatus, the rest
//! from the ACPI battery classes in `root\wmi` (the same _BIF/_BST data
//! Linux shows in sysfs, in mWh and mV).

use super::{BatterySnapshot, Energy, Reading, Voltage};
use crate::wmi::{Namespace, Object};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

const NAMESPACE: &str = r"root\wmi";
const AC_ONLINE: u8 = 1;
const NO_SYSTEM_BATTERY: u8 = 128;
/// BatteryFlag and BatteryLifePercent when Windows can't tell.
const UNKNOWN: u8 = 255;
/// BATTERY_UNKNOWN_RATE (0x80000000)
const UNKNOWN_RATE: i32 = i32::MIN;
const MICRO_PER_MILLI: i64 = 1_000;
const MILLI_PER_UNIT: f64 = 1_000.0;
const SNAPSHOT_ROW_TIMEOUT_MS: i32 = 750;

fn system_status() -> Option<SYSTEM_POWER_STATUS> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status).ok()? };
    Some(status)
}

/// The first instance of a `root\wmi` battery class.
fn battery_class(class: &str) -> Option<Object> {
    Namespace::connect(NAMESPACE)
        .ok()?
        .first(class)
        .ok()
        .flatten()
}

/// An unknown flag (255) has the no-battery bit set too: not an answer.
fn has_battery(status: &SYSTEM_POWER_STATUS) -> bool {
    status.BatteryFlag == UNKNOWN || status.BatteryFlag & NO_SYSTEM_BATTERY == 0
}

pub fn present() -> bool {
    system_status().is_some_and(|s| has_battery(&s))
}

pub fn capacity() -> Option<u8> {
    capacity_from(&system_status()?)
}

fn capacity_from(status: &SYSTEM_POWER_STATUS) -> Option<u8> {
    (has_battery(status) && status.BatteryLifePercent != UNKNOWN)
        .then_some(status.BatteryLifePercent)
}

pub fn energy() -> Option<Energy> {
    energy_from(&battery_class("BatteryStatus")?)
}

fn energy_from(battery: &Object) -> Option<Energy> {
    battery
        .u32("RemainingCapacity")
        .map(|mwh| Energy::from_mwh(mwh.into()))
}

pub fn voltage() -> Option<Voltage> {
    voltage_from(&battery_class("BatteryStatus")?)
}

fn voltage_from(battery: &Object) -> Option<Voltage> {
    battery
        .u32("Voltage")
        .map(|mv| Voltage::from_uv(i64::from(mv) * MICRO_PER_MILLI))
}

/// In sysfs terms: Charging, Discharging, Not charging (on AC, no charge
/// flowing) or Unknown.
pub fn status() -> Option<String> {
    status_from(&battery_class("BatteryStatus")?)
}

fn status_from(battery: &Object) -> Option<String> {
    let status = if battery.bool("Charging")? {
        "Charging"
    } else if battery.bool("Discharging")? {
        "Discharging"
    } else if battery.bool("PowerOnline")? {
        "Not charging"
    } else {
        "Unknown"
    };
    Some(status.into())
}

pub fn health() -> Option<f64> {
    health_from(
        &battery_class("BatteryFullChargedCapacity")?,
        &battery_class("BatteryStaticData")?,
    )
}

fn health_from(full: &Object, design: &Object) -> Option<f64> {
    let full = full.u32("FullChargedCapacity")?;
    let design = design.u32("DesignedCapacity")?;
    (design > 0).then(|| f64::from(full) / f64::from(design) * 100.0)
}

pub fn cycles() -> Option<u64> {
    battery_class("BatteryCycleCount")?
        .u32("CycleCount")
        .map(u64::from)
}

/// Whichever of charge or discharge rate (mW, CIM sint32) is flowing; a
/// driver may report it negative, or as i32::MIN when unknown.
pub fn power_watts() -> Option<f64> {
    power_watts_from(&battery_class("BatteryStatus")?)
}

fn power_watts_from(battery: &Object) -> Option<f64> {
    let rate = |name| {
        battery
            .i32(name)
            .filter(|&mw| mw != UNKNOWN_RATE)
            .map_or(0, i32::unsigned_abs)
    };
    Some(f64::from(rate("ChargeRate").max(rate("DischargeRate"))) / MILLI_PER_UNIT)
}

pub fn manufacturer() -> Option<String> {
    battery_class("BatteryStaticData")?.string("ManufactureName")
}

pub fn model() -> Option<String> {
    battery_class("BatteryStaticData")?.string("DeviceName")
}

/// None when Windows can't tell (ACLineStatus 255).
pub fn ac_status() -> Option<bool> {
    match system_status()?.ACLineStatus {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub fn on_ac() -> bool {
    system_status().is_some_and(|s| s.ACLineStatus == AC_ONLINE)
}

pub fn battery_snapshot() -> BatterySnapshot {
    let system = system_status();
    let mut snapshot = BatterySnapshot {
        capacity: system.as_ref().and_then(capacity_from),
        on_ac: system.is_some_and(|status| status.ACLineStatus == AC_ONLINE),
        ..Default::default()
    };
    let Ok(namespace) = Namespace::connect(NAMESPACE) else {
        return snapshot;
    };
    let first = |class| {
        namespace
            .first_with_timeout(class, SNAPSHOT_ROW_TIMEOUT_MS)
            .ok()
            .flatten()
    };
    if let Some(battery) = first("BatteryStatus") {
        snapshot.status = status_from(&battery);
        snapshot.energy = energy_from(&battery);
        snapshot.voltage = voltage_from(&battery);
        snapshot.power_watts = power_watts_from(&battery);
    }
    if let Some(data) = first("BatteryStaticData") {
        snapshot.manufacturer = data.string("ManufactureName");
        snapshot.model = data.string("DeviceName");
        snapshot.health =
            first("BatteryFullChargedCapacity").and_then(|full| health_from(&full, &data));
    }
    snapshot.cycles = first("BatteryCycleCount")
        .and_then(|battery| battery.u32("CycleCount"))
        .map(u64::from);
    snapshot
}

pub fn reading() -> Option<Reading> {
    Some(Reading {
        capacity: capacity()?,
        on_ac: on_ac(),
    })
}
