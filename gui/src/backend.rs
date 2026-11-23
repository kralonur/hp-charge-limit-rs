use hp_charge_limit_desktop::{self as desktop, Action, ChargeSettings, ServiceStatus};

#[derive(Default)]
pub struct Snapshot {
    pub capacity: Option<u8>,
    pub on_ac: bool,
    pub status: String,
    pub health: String,
    pub cycles: String,
    pub watts: String,
    pub energy: String,
    pub voltage: String,
    pub pack: String,
    pub service_state: ServiceStatus,
    pub cli_error: Option<String>,
    /// Blocker for setup if missing, or for start if stopped.
    pub prerequisite: Option<String>,
    #[cfg(target_os = "linux")]
    pub driver_hint: Option<String>,
    pub settings: Option<Result<ChargeSettings, String>>,
    pub startup: Option<Result<bool, String>>,
    pub removable: bool,
    pub bypass: bool,
    pub last_off: Option<String>,
    pub next_check_unix_ms: Option<u64>,
}

impl Snapshot {
    pub fn read() -> Self {
        let data = match desktop::status_snapshot() {
            Ok(data) => data,
            Err(error) => {
                return Self {
                    service_state: ServiceStatus::Unavailable("CLI unavailable".into()),
                    status: "Battery unavailable".into(),
                    health: unknown(),
                    cycles: unknown(),
                    watts: unknown(),
                    energy: unknown(),
                    voltage: unknown(),
                    cli_error: Some(error),
                    ..Default::default()
                };
            }
        };
        let battery = data.battery;
        let mut snapshot = Self {
            capacity: battery.capacity,
            on_ac: battery.on_ac,
            status: battery
                .status
                .unwrap_or_else(|| "Battery unavailable".into()),
            health: battery.health.map_or_else(unknown, |v| format!("{v:.1}%")),
            cycles: battery.cycles.map_or_else(unknown, |v| v.to_string()),
            watts: battery
                .power_watts
                .map_or_else(unknown, |v| format!("{v:.1} W")),
            energy: battery
                .energy_uwh
                .map_or_else(unknown, |v| format!("{:.2} Wh", v as f64 / 1e6)),
            voltage: battery
                .voltage_uv
                .map_or_else(unknown, |v| format!("{:.2} V", v as f64 / 1e6)),
            pack: [battery.manufacturer, battery.model]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
            service_state: data.service,
            cli_error: None,
            prerequisite: None,
            #[cfg(target_os = "linux")]
            driver_hint: data.driver_hint,
            settings: data.settings,
            startup: data.startup,
            removable: data.removable,
            bypass: data.bypass,
            last_off: data.last_off,
            next_check_unix_ms: data.next_check_unix_ms,
        };
        snapshot.prerequisite = match snapshot.service_state {
            ServiceStatus::Missing => snapshot.action_blocker(Action::Setup(None)),
            ServiceStatus::Stopped => snapshot.action_blocker(Action::Start),
            _ => None,
        };
        if matches!(
            snapshot.service_state,
            ServiceStatus::Running | ServiceStatus::Stopped
        ) && snapshot.settings.as_ref().is_some_and(Result::is_ok)
            && let Some(message) = desktop::authentication_prerequisite()
        {
            snapshot.settings = Some(Err(message));
        }
        snapshot
    }

    pub fn action_blocker(&self, action: Action) -> Option<String> {
        if let Some(message) = self
            .cli_error
            .clone()
            .or_else(desktop::authentication_prerequisite)
        {
            return Some(message);
        }
        #[cfg(windows)]
        let _ = action;
        #[cfg(target_os = "linux")]
        if action.requires_loaded_module() {
            return self.driver_hint.clone();
        }
        None
    }
}

fn unknown() -> String {
    "—".into()
}
