//! `hp-charge-limit-rs status`: one look at battery, AC, firmware and service.

use crate::firmware::Wmi;
use crate::power::{self, Energy};
use crate::{bypass, offstate, service};
use anyhow::{Context, Result};
use hp_charge_limit_protocol::{
    API_VERSION, BatteryStatus, ServiceStatus, StatusSnapshot, WatchResult,
};
use std::io::{self, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};

const WATCH_SAMPLE: Duration = Duration::from_secs(5);
/// Reported energy moves in 10 mWh steps (sysfs energy_now; Windows
/// RemainingCapacity is often coarser); smaller changes are noise.
const ENERGY_RESOLUTION: Energy = Energy::from_mwh(10);

/// Desktop polling is unprivileged and never attempts firmware readback.
pub(crate) fn write_json(brief: bool) -> Result<()> {
    let (service, pid) = service::desktop_status();
    let installed = matches!(service, ServiceStatus::Running | ServiceStatus::Stopped);
    let battery = if brief {
        BatteryStatus {
            capacity: power::capacity(),
            on_ac: power::on_ac(),
            ..Default::default()
        }
    } else {
        let battery = power::battery_snapshot();
        BatteryStatus {
            capacity: battery.capacity,
            on_ac: battery.on_ac,
            status: battery.status,
            health: battery.health,
            cycles: battery.cycles,
            power_watts: battery.power_watts,
            energy_uwh: battery.energy.map(power::Energy::uwh),
            voltage_uv: battery.voltage.map(power::Voltage::uv),
            manufacturer: battery.manufacturer,
            model: battery.model,
        }
    };
    let snapshot = StatusSnapshot {
        api_version: API_VERSION,
        battery,
        bypass: bypass::enabled(),
        settings: (!brief && (installed || service == ServiceStatus::Missing))
            .then(|| service::charge_settings(installed).map_err(|error| format!("{error:#}"))),
        startup: (!brief && installed)
            .then(|| service::startup_enabled().map_err(|error| format!("{error:#}"))),
        removable: !brief && installed && service::can_uninstall(),
        last_off: (!brief).then(offstate::last).flatten(),
        next_check_unix_ms: if brief {
            None
        } else {
            pid.and_then(crate::daemon::next_check_unix_ms)
        },
        #[cfg(target_os = "linux")]
        driver_hint: (!brief).then(driver_hint).flatten(),
        service,
    };
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &snapshot)?;
    writeln!(out)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn driver_hint() -> Option<String> {
    if crate::paths::at(crate::paths::ACPI_CALL).exists() {
        return None;
    }
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let hint = if release
        .lines()
        .any(|line| line == "ID=arch" || line == "ID=endeavouros")
    {
        "Install acpi_call (matching stock kernel), or acpi_call-dkms plus matching kernel headers."
    } else if release.contains("debian") || release.contains("ubuntu") {
        "Install acpi-call-dkms and matching linux-headers for your running kernel."
    } else {
        "Install your distribution's acpi_call package and matching kernel headers if using DKMS."
    };
    Some(format!(
        "acpi_call is not loaded. {hint} Then select Load ACPI driver. Secure Boot may require signing the module."
    ))
}

pub fn show(watch_secs: Option<u64>) -> Result<()> {
    let unknown = || "?".to_string();
    println!(
        "AC:       {}",
        if power::on_ac() { "online" } else { "offline" }
    );
    println!(
        "battery:  {}%  {}  {}  {}",
        power::capacity().map_or_else(unknown, |c| c.to_string()),
        power::status().unwrap_or_else(unknown),
        power::energy().map_or_else(unknown, |e| e.to_string()),
        power::voltage().map_or_else(unknown, |v| v.to_string()),
    );
    println!(
        "pack:     {}  {}",
        power::manufacturer().unwrap_or_else(unknown),
        power::model().unwrap_or_else(unknown),
    );
    println!(
        "health:   {}  cycles: {}  power: {}",
        power::health().map_or_else(unknown, |h| format!("{h:.1}%")),
        power::cycles().map_or_else(unknown, |c| c.to_string()),
        power::power_watts().map_or_else(unknown, |p| format!("{p:.2} W")),
    );
    println!(
        "bypass:   {}",
        if bypass::enabled() {
            "enabled (requested)"
        } else {
            "disabled"
        }
    );
    match Wmi.read() {
        Ok(mode) => {
            println!("firmware: {mode}");
            if mode.is_hold() {
                println!(
                    "          (in hold, LED and status show charging; some boards still charge slowly: check with --watch)"
                );
            }
        }
        Err(e) => println!("firmware: unreadable: {e:#}"),
    }
    println!(
        "service:  {} {}",
        service::SERVICE,
        service::service_state()
    );
    if let Some(line) = offstate::last() {
        println!("last off: {line}");
    }
    if let Some(secs) = watch_secs {
        let result = watch(secs, true)?;
        println!(
            "result: {} ({:+} mWh in {} s)",
            result.flow, result.delta_mwh, result.seconds
        );
    }
    Ok(())
}

pub(crate) fn write_watch_json(seconds: u64) -> Result<()> {
    let result = watch(seconds, false)?;
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &result)?;
    writeln!(out)?;
    Ok(())
}

/// Sample remaining energy rather than relying on the LED or charge status.
fn watch(seconds: u64, report_samples: bool) -> Result<WatchResult> {
    let span = Duration::from_secs(seconds);
    let start = power::energy().context("battery energy unreadable")?;
    if report_samples {
        let t0 = Instant::now();
        println!("watching energy for {seconds} s ...");
        while t0.elapsed() < span {
            sleep(WATCH_SAMPLE.min(span.saturating_sub(t0.elapsed())));
            let energy = power::energy().unwrap_or(start);
            println!(
                "  +{:>3} s  {energy}  ({:+} mWh)",
                t0.elapsed().as_secs(),
                (energy - start).mwh()
            );
        }
    } else {
        sleep(span);
    }
    let delta =
        power::energy().context("battery energy unreadable at the end of the check")? - start;
    Ok(WatchResult {
        flow: charge_flow(delta).into(),
        delta_mwh: delta.mwh(),
        seconds,
    })
}

fn charge_flow(delta: Energy) -> &'static str {
    if delta.abs() <= ENERGY_RESOLUTION {
        "no charge flow"
    } else if delta > Energy::from_uwh(0) {
        "CHARGING"
    } else {
        "DISCHARGING"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charge_flow_respects_sensor_resolution() {
        for (delta, expected) in [
            (-11, "DISCHARGING"),
            (-10, "no charge flow"),
            (10, "no charge flow"),
            (11, "CHARGING"),
        ] {
            assert_eq!(charge_flow(Energy::from_mwh(delta)), expected);
        }
    }
}
