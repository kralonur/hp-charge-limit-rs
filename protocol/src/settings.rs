use anyhow::{Result, ensure};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_PERCENT: i64 = 100;
pub const DEFAULT_STOP: u8 = 50;
pub const DEFAULT_START: u8 = 40;
pub const DEFAULT_INTERVAL_SECS: u64 = 30;
pub const MIN_INTERVAL_SECS: u64 = 5;
pub const DEFAULT_OFF_MIN: u8 = 40;
pub const DEFAULT_RUN_MODE: Mode = Mode::Inhibit;
pub const DEFAULT_OFF_MODE: StopAction = StopAction::Hold;
pub const DEFAULT_STOP_MODE: StopAction = StopAction::Auto;

/// The supported writable modes, as their SBCC mode dword.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum Mode {
    /// Normal charging; clears WACL/SBIS/SBTC/WPBS
    Auto = 0x0000,
    /// SBIS: no charging, white LED; the EC clears it at shutdown
    Inhibit = 0x0500,
    /// SBTC: no charging, survives shutdown; LED/status still say charging
    Hold = 0x0300,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value = self.to_possible_value().expect("no skipped variants");
        f.write_str(value.get_name())
    }
}

/// What to do with the firmware mode when the limiter ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
            Self::Auto => Some(Mode::Auto),
            Self::Inhibit => Some(Mode::Inhibit),
            Self::Hold => Some(Mode::Hold),
            Self::Keep => None,
        }
    }
}

impl fmt::Display for StopAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value = self.to_possible_value().expect("no skipped variants");
        f.write_str(value.get_name())
    }
}

#[derive(Args, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvancedSettings {
    /// Poll interval in seconds
    #[arg(long, requires = "start", value_parser = clap::value_parser!(u64).range(MIN_INTERVAL_SECS..))]
    pub interval: Option<u64>,
    /// Firmware mode used when the stop percentage is reached
    #[arg(long, value_enum, requires = "start")]
    pub run_mode: Option<Mode>,
    /// Allow normal charging on shutdown at or below this %
    #[arg(long, requires = "start", value_parser = clap::value_parser!(u8).range(0..=MAX_PERCENT))]
    pub off_min: Option<u8>,
    /// Firmware mode used when stopping only the service
    #[arg(long, value_enum, requires = "start")]
    pub stop_mode: Option<StopAction>,
    /// Broadcast Linux desktop notification events
    #[cfg(target_os = "linux")]
    #[arg(long, requires = "start")]
    pub notify: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChargeSettings {
    pub start: u8,
    pub stop: u8,
    pub off_mode: StopAction,
    /// Unspecified fields are preserved when configuring an existing service.
    pub advanced: AdvancedSettings,
}

impl Default for ChargeSettings {
    fn default() -> Self {
        Self {
            start: DEFAULT_START,
            stop: DEFAULT_STOP,
            off_mode: DEFAULT_OFF_MODE,
            advanced: AdvancedSettings {
                interval: Some(DEFAULT_INTERVAL_SECS),
                run_mode: Some(DEFAULT_RUN_MODE),
                off_min: Some(DEFAULT_OFF_MIN),
                stop_mode: Some(DEFAULT_STOP_MODE),
                #[cfg(target_os = "linux")]
                notify: Some(true),
            },
        }
    }
}

impl ChargeSettings {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.stop <= MAX_PERCENT as u8 && self.start < self.stop,
            "Resume percentage must be less than stop percentage, within 0–100%."
        );
        ensure!(
            self.advanced
                .interval
                .is_none_or(|value| value >= MIN_INTERVAL_SECS),
            "Poll interval must be at least {MIN_INTERVAL_SECS} seconds."
        );
        ensure!(
            self.advanced
                .off_min
                .is_none_or(|value| value <= MAX_PERCENT as u8),
            "The shutdown exception must be within 0–100%."
        );
        Ok(())
    }

    pub fn values(self) -> Vec<(&'static str, String)> {
        let mut values = vec![
            ("start", self.start.to_string()),
            ("stop", self.stop.to_string()),
            ("off-mode", self.off_mode.to_string()),
        ];
        let advanced = self.advanced;
        values.extend(
            [
                ("interval", advanced.interval.map(|value| value.to_string())),
                ("run-mode", advanced.run_mode.map(|value| value.to_string())),
                ("off-min", advanced.off_min.map(|value| value.to_string())),
                (
                    "stop-mode",
                    advanced.stop_mode.map(|value| value.to_string()),
                ),
                #[cfg(target_os = "linux")]
                ("notify", advanced.notify.map(|value| value.to_string())),
            ]
            .into_iter()
            .filter_map(|(key, value)| Some((key, value?))),
        );
        values
    }

    pub fn to_args(self) -> Vec<String> {
        self.values()
            .into_iter()
            .map(|(key, value)| format!("--{key}={value}"))
            .collect()
    }
}
