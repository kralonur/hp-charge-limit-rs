//! Data and argument types shared by the CLI and its desktop clients.
//! No platform or service operations belong here.

mod settings;

use serde::{Deserialize, Serialize};
pub use settings::*;

pub const API_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Running,
    Starting,
    Stopped,
    #[default]
    Missing,
    Unavailable(String),
}

impl ServiceStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Running => "active",
            Self::Starting => "starting",
            Self::Stopped => "stopped",
            Self::Missing => "not installed",
            Self::Unavailable(message) => message,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BatteryStatus {
    pub capacity: Option<u8>,
    pub on_ac: bool,
    pub status: Option<String>,
    pub health: Option<f64>,
    pub cycles: Option<u64>,
    pub power_watts: Option<f64>,
    pub energy_uwh: Option<i64>,
    pub voltage_uv: Option<i64>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub api_version: u32,
    pub battery: BatteryStatus,
    pub bypass: bool,
    pub service: ServiceStatus,
    pub settings: Option<Result<ChargeSettings, String>>,
    pub startup: Option<Result<bool, String>>,
    pub removable: bool,
    pub last_off: Option<String>,
    /// Deadline from the running daemon; absent for older services or brief status.
    pub next_check_unix_ms: Option<u64>,
    #[cfg(target_os = "linux")]
    pub driver_hint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WatchResult {
    pub flow: String,
    pub delta_mwh: i64,
    pub seconds: u64,
}
