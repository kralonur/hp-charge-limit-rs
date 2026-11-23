//! Command line. Daemon options also read environment variables, which is
//! how the Linux service reads /etc/hp-charge-limit-rs.conf; the Windows
//! service gets them as arguments (`service install`). Fresh Windows setup
//! uses the defaults below; Linux setup preserves any existing config file.

use anyhow::{Result, ensure};
#[cfg(target_os = "linux")]
use clap::ArgAction;
use clap::{Args, Parser, Subcommand, ValueEnum};
pub use clap_complete::Shell;
pub use hp_charge_limit_protocol::{AdvancedSettings, ChargeSettings, Mode, StopAction};
use hp_charge_limit_protocol::{
    DEFAULT_INTERVAL_SECS, DEFAULT_OFF_MIN, DEFAULT_OFF_MODE, DEFAULT_RUN_MODE, DEFAULT_START,
    DEFAULT_STOP, DEFAULT_STOP_MODE, MAX_PERCENT, MIN_INTERVAL_SECS,
};

#[derive(Parser)]
#[command(
    version,
    about = "Battery charge limiter using the HP WMI charge-control method SBCC"
)]
pub struct Cli {
    #[cfg(windows)]
    #[arg(long, hide = true, global = true)]
    pub desktop_report: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Show battery, AC and firmware charge mode
    Status {
        /// Also sample energy for this many seconds to see if charge really flows
        #[arg(long, value_name = "SECS")]
        watch: Option<u64>,
        /// Emit machine-readable battery and service status (or just the result with --watch)
        #[arg(long)]
        json: bool,
        /// Only query battery percentage, AC, bypass and service state
        #[arg(long, requires = "json", conflicts_with = "watch")]
        brief: bool,
    },
    /// Diagnose supplies, firmware access and readback, and service setup
    Diagnose,
    /// Show desktop notifications for daemon D-Bus signals (run as your user, no root)
    #[cfg(target_os = "linux")]
    Notify,
    /// Set a firmware charge mode once
    Set {
        mode: Mode,
        /// Refuse to change the mode while the service is running
        #[arg(long)]
        require_idle: bool,
    },
    /// Read the current firmware charge mode
    FirmwareStatus,
    /// Show recent service logs
    Logs,
    /// Load the already-installed ACPI driver
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    LoadDriver,
    /// Pause or resume charge limiting until reboot (applied on the next daemon poll)
    Bypass { action: BypassAction },
    /// Run the limiter loop (what the service runs)
    Daemon(DaemonOpts),
    /// Set up, start or manage the desktop service (run as root/administrator)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Generate a shell completion script (no root required)
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum BypassAction {
    On,
    Off,
}

#[derive(Subcommand)]
pub enum ServiceAction {
    /// Create a fresh service; preserve config files and refuse an existing service
    Setup {
        #[command(flatten)]
        settings: ChargeSettingsArgs,
    },
    /// Change charge thresholds and shutdown mode on a supported installed service
    #[command(arg_required_else_help = true)]
    Configure {
        #[command(flatten)]
        settings: ChargeSettingsArgs,
    },
    /// Start the installed service without changing its settings
    Start,
    /// Stop the service without changing its startup settings
    Stop,
    /// Start the service automatically at boot (does not start it now)
    Enable,
    /// Turn off automatic startup (does not stop the service now)
    Disable,
    #[cfg(windows)]
    /// Install (or reconfigure) the service with these daemon options, start at boot, and start it now
    Install(DaemonOpts),
    /// Replace the service executable with this version, without changing settings
    #[cfg(windows)]
    Upgrade,
    /// Stop and remove the app-managed service
    Uninstall,
}

#[derive(Args)]
pub struct ChargeSettingsArgs {
    /// Resume charging at or below this %
    #[arg(long, requires_all = ["stop", "off_mode"], value_parser = clap::value_parser!(u8).range(0..MAX_PERCENT))]
    start: Option<u8>,
    /// Stop charging at or above this %
    #[arg(long, requires_all = ["start", "off_mode"], value_parser = clap::value_parser!(u8).range(1..=MAX_PERCENT))]
    stop: Option<u8>,
    /// Firmware mode requested on shutdown or reboot
    #[arg(long, value_enum, requires_all = ["start", "stop"])]
    off_mode: Option<StopAction>,
    #[command(flatten)]
    advanced: AdvancedSettings,
}

impl ChargeSettingsArgs {
    pub(crate) fn settings(&self) -> Option<ChargeSettings> {
        Some(ChargeSettings {
            start: self.start?,
            stop: self.stop?,
            off_mode: self.off_mode?,
            advanced: self.advanced,
        })
    }
}

#[derive(Args, Clone)]
pub struct DaemonOpts {
    /// Stop charging at or above this %
    #[arg(long, env = "HP_CHARGE_LIMIT_RS_STOP", default_value_t = DEFAULT_STOP,
          value_parser = clap::value_parser!(u8).range(1..=MAX_PERCENT))]
    pub stop: u8,
    /// Resume charging at or below this %
    #[arg(long, env = "HP_CHARGE_LIMIT_RS_START", default_value_t = DEFAULT_START,
          value_parser = clap::value_parser!(u8).range(0..MAX_PERCENT))]
    pub start: u8,
    /// Poll interval in seconds
    #[arg(long, env = "HP_CHARGE_LIMIT_RS_INTERVAL", default_value_t = DEFAULT_INTERVAL_SECS,
          value_parser = clap::value_parser!(u64).range(MIN_INTERVAL_SECS..))]
    pub interval: u64,
    /// Mode used while running once --stop is reached
    #[arg(long, env = "HP_CHARGE_LIMIT_RS_RUN_MODE", value_enum, default_value_t = DEFAULT_RUN_MODE)]
    pub run_mode: Mode,
    /// Mode set when the PC shuts down or reboots
    #[arg(long, env = "HP_CHARGE_LIMIT_RS_OFF_MODE", value_enum, default_value_t = DEFAULT_OFF_MODE)]
    pub off_mode: StopAction,
    /// At shutdown, use auto instead of --off-mode at or below this % (charge while off)
    #[arg(long, env = "HP_CHARGE_LIMIT_RS_OFF_MIN", default_value_t = DEFAULT_OFF_MIN,
          value_parser = clap::value_parser!(u8).range(0..=MAX_PERCENT))]
    pub off_min: u8,
    /// Mode set when only the service is stopped, not the PC
    #[arg(long, env = "HP_CHARGE_LIMIT_RS_STOP_MODE", value_enum, default_value_t = DEFAULT_STOP_MODE)]
    pub stop_mode: StopAction,
    /// Broadcast D-Bus signals for desktop notifications; false never touches the bus
    #[cfg(target_os = "linux")]
    #[arg(long, env = "HP_CHARGE_LIMIT_RS_NOTIFY", default_value_t = true, action = ArgAction::Set)]
    pub notify: bool,
}

impl Default for DaemonOpts {
    fn default() -> Self {
        Self {
            stop: DEFAULT_STOP,
            start: DEFAULT_START,
            interval: DEFAULT_INTERVAL_SECS,
            run_mode: DEFAULT_RUN_MODE,
            off_mode: DEFAULT_OFF_MODE,
            off_min: DEFAULT_OFF_MIN,
            stop_mode: DEFAULT_STOP_MODE,
            #[cfg(target_os = "linux")]
            notify: true,
        }
    }
}

impl DaemonOpts {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.start < self.stop,
            "--start ({}) must be below --stop ({})",
            self.start,
            self.stop
        );
        Ok(())
    }

    #[cfg(windows)]
    pub(crate) fn settings(&self) -> ChargeSettings {
        ChargeSettings {
            start: self.start,
            stop: self.stop,
            off_mode: self.off_mode,
            advanced: AdvancedSettings {
                interval: Some(self.interval),
                run_mode: Some(self.run_mode),
                off_min: Some(self.off_min),
                stop_mode: Some(self.stop_mode),
                #[cfg(target_os = "linux")]
                notify: Some(self.notify),
            },
        }
    }

    #[cfg(windows)]
    pub(crate) fn apply_settings(&mut self, settings: ChargeSettings) {
        self.start = settings.start;
        self.stop = settings.stop;
        self.off_mode = settings.off_mode;
        self.interval = settings.advanced.interval.unwrap_or(self.interval);
        self.run_mode = settings.advanced.run_mode.unwrap_or(self.run_mode);
        self.off_min = settings.advanced.off_min.unwrap_or(self.off_min);
        self.stop_mode = settings.advanced.stop_mode.unwrap_or(self.stop_mode);
    }

    /// The same options as `daemon` arguments, for the service command line.
    #[cfg(windows)]
    pub fn to_args(&self) -> Vec<String> {
        vec![
            format!("--stop={}", self.stop),
            format!("--start={}", self.start),
            format!("--interval={}", self.interval),
            format!("--run-mode={}", self.run_mode),
            format!("--off-mode={}", self.off_mode),
            format!("--off-min={}", self.off_min),
            format!("--stop-mode={}", self.stop_mode),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_setup_accepts_defaults_or_a_complete_charge_band() {
        let setup = Cli::try_parse_from(["hp-charge-limit-rs", "service", "setup"]).unwrap();
        assert!(matches!(
            setup.cmd,
            Cmd::Service {
                action: ServiceAction::Setup { settings }
            } if settings.settings().is_none()
        ));
        assert!(
            Cli::try_parse_from(["hp-charge-limit-rs", "service", "setup", "--stop", "80"])
                .is_err()
        );
        let settings = ChargeSettings {
            start: 75,
            stop: 80,
            off_mode: StopAction::Inhibit,
            advanced: AdvancedSettings {
                interval: Some(17),
                off_min: Some(10),
                ..AdvancedSettings::default()
            },
        };
        for (command, arg) in [("setup", true), ("configure", false)] {
            let cli = Cli::try_parse_from(
                ["hp-charge-limit-rs", "service", command]
                    .map(str::to_owned)
                    .into_iter()
                    .chain(settings.to_args()),
            )
            .unwrap();
            match cli.cmd {
                Cmd::Service {
                    action: ServiceAction::Setup { settings: parsed },
                } if arg => assert_eq!(parsed.settings(), Some(settings)),
                Cmd::Service {
                    action: ServiceAction::Configure { settings: parsed },
                } if !arg => assert_eq!(parsed.settings(), Some(settings)),
                _ => panic!("unexpected command"),
            }
        }

        let start = Cli::try_parse_from(["hp-charge-limit-rs", "service", "start"]).unwrap();
        assert!(matches!(
            start.cmd,
            Cmd::Service {
                action: ServiceAction::Start
            }
        ));
    }
}
