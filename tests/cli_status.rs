#![cfg(target_os = "linux")]

use hp_charge_limit_protocol::{API_VERSION, StatusSnapshot, WatchResult};
use std::fs;
use std::process::Command;

#[test]
fn status_json_reads_battery_and_bypass_without_firmware_access() {
    let root = tempfile::tempdir().unwrap();
    let fixture = |path: &str, value: &str| {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    };
    for (name, value) in [
        ("type", "Battery"),
        ("capacity", "74"),
        ("status", "Not charging"),
        ("energy_now", "26000000"),
        ("voltage_now", "11490000"),
    ] {
        fixture(&format!("sys/class/power_supply/BAT1/{name}"), value);
    }
    fixture("sys/class/power_supply/AC/type", "Mains");
    fixture("sys/class/power_supply/AC/online", "1");
    fixture("run/hp-charge-limit-rs.bypass", "");
    fixture(
        "var/lib/hp-charge-limit-rs/off-state.last",
        "off 12 min in hold",
    );
    fixture("proc/acpi/call", "unchanged");

    let status = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_hp-charge-limit-rs"))
            .env("HP_CHARGE_LIMIT_RS_TEST_ROOT", root.path())
            .args(["status", "--json"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    let full: StatusSnapshot = serde_json::from_slice(&status(&[])).unwrap();
    assert_eq!(full.api_version, API_VERSION);
    assert_eq!(full.battery.capacity, Some(74));
    assert_eq!(full.battery.status.as_deref(), Some("Not charging"));
    assert_eq!(full.battery.energy_uwh, Some(26_000_000));
    assert_eq!(full.battery.voltage_uv, Some(11_490_000));
    assert!(full.battery.on_ac && full.bypass);
    assert_eq!(full.last_off.as_deref(), Some("off 12 min in hold"));
    assert!(full.driver_hint.is_none());

    let brief: StatusSnapshot = serde_json::from_slice(&status(&["--brief"])).unwrap();
    assert_eq!(brief.battery.capacity, full.battery.capacity);
    assert!(brief.battery.on_ac && brief.bypass);
    assert!(brief.battery.energy_uwh.is_none());
    assert!(brief.settings.is_none() && brief.startup.is_none() && brief.last_off.is_none());
    assert!(brief.next_check_unix_ms.is_none());

    let watch: WatchResult = serde_json::from_slice(&status(&["--watch", "1"])).unwrap();
    assert_eq!(watch.flow, "no charge flow");
    assert_eq!(watch.delta_mwh, 0);
    assert_eq!(watch.seconds, 1);
    assert_eq!(
        fs::read_to_string(root.path().join("proc/acpi/call")).unwrap(),
        "unchanged"
    );
}
