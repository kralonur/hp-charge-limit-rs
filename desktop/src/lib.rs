//! Shared, non-GPU desktop operations. The CLI never depends on this crate.
use hp_charge_limit_protocol::API_VERSION;
pub use hp_charge_limit_protocol::{
    ChargeSettings, Mode, ServiceStatus, StatusSnapshot, StopAction, WatchResult,
};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
mod platform_windows;

#[derive(Clone)]
pub struct TrayState {
    pub capacity: Option<u8>,
    pub bypass: bool,
    pub service: ServiceStatus,
}

pub fn tray_state() -> TrayState {
    match cli_path().and_then(|path| read_status(&path, true)) {
        Ok(status) => TrayState {
            capacity: status.battery.capacity,
            bypass: status.bypass,
            service: status.service,
        },
        Err(message) => TrayState {
            capacity: None,
            bypass: false,
            service: ServiceStatus::Unavailable(message),
        },
    }
}

pub fn status_snapshot() -> Result<StatusSnapshot, String> {
    read_status(&cli_path()?, false)
}

fn read_status(cli: &Path, brief: bool) -> Result<StatusSnapshot, String> {
    let mut command = unprivileged_command(cli);
    command.args(["status", "--json"]);
    if brief {
        command.arg("--brief");
    }
    let output = command.output().map_err(|error| {
        format!(
            "Could not run {}: {error}. Reinstall HP Charge Limit Desktop.",
            cli.display()
        )
    })?;
    let unsupported = "The CLI is incompatible with this desktop app. Reinstall the desktop bundle or update your CLI package.";
    if !output.status.success() {
        return Err(unsupported.into());
    }
    decode_status(&output.stdout).ok_or_else(|| unsupported.into())
}

fn decode_status(stdout: &[u8]) -> Option<StatusSnapshot> {
    let status: StatusSnapshot = serde_json::from_slice(stdout).ok()?;
    (status.api_version == API_VERSION).then_some(status)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Diagnose,
    Setup(Option<ChargeSettings>),
    Start,
    Stop,
    Startup(bool),
    Uninstall,
    #[cfg(windows)]
    Upgrade,
    Configure(ChargeSettings),
    Bypass(bool),
    ReadMode,
    SetMode(Mode),
    Logs,
    #[cfg(target_os = "linux")]
    LoadDriver,
}

impl Action {
    #[cfg(target_os = "linux")]
    pub fn requires_loaded_module(self) -> bool {
        matches!(self, Self::Setup(_) | Self::ReadMode | Self::SetMode(_))
    }

    fn arguments(self) -> Vec<String> {
        let (command, settings): (&[&str], Option<ChargeSettings>) = match self {
            Self::Diagnose => (&["diagnose"], None),
            Self::Setup(settings) => (&["service", "setup"], settings),
            Self::Start => (&["service", "start"], None),
            Self::Stop => (&["service", "stop"], None),
            Self::Startup(true) => (&["service", "enable"], None),
            Self::Startup(false) => (&["service", "disable"], None),
            Self::Uninstall => (&["service", "uninstall"], None),
            #[cfg(windows)]
            Self::Upgrade => (&["service", "upgrade"], None),
            Self::ReadMode => (&["firmware-status"], None),
            Self::SetMode(mode) => {
                return vec!["set".into(), mode.to_string(), "--require-idle".into()];
            }
            Self::Logs => (&["logs"], None),
            #[cfg(target_os = "linux")]
            Self::LoadDriver => (&["load-driver"], None),
            Self::Configure(settings) => (&["service", "configure"], Some(settings)),
            Self::Bypass(true) => (&["bypass", "on"], None),
            Self::Bypass(false) => (&["bypass", "off"], None),
        };
        command
            .iter()
            .map(|arg| (*arg).to_owned())
            .chain(settings.into_iter().flat_map(ChargeSettings::to_args))
            .collect()
    }
}

fn sibling(name: &str) -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .map(|parent| parent.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
        .ok_or_else(|| "Could not locate the desktop installation".into())
}

/// Use the matching sibling first; distro packages may share a system CLI.
fn cli_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    let installed = PathBuf::from("/usr/bin/hp-charge-limit-rs");
    #[cfg(windows)]
    let installed = PathBuf::from(
        std::env::var_os("ProgramW6432")
            .or_else(|| std::env::var_os("ProgramFiles"))
            .unwrap_or_else(|| r"C:\Program Files".into()),
    )
    .join("hp-charge-limit-rs/hp-charge-limit-rs.exe");
    select_cli(sibling("hp-charge-limit-rs")?, installed)
}

fn select_cli(bundled: PathBuf, installed: PathBuf) -> Result<PathBuf, String> {
    [bundled, installed]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "CLI not found. Reinstall HP Charge Limit Desktop or install the matching core package."
                .into()
        })
}

pub fn authentication_prerequisite() -> Option<String> {
    #[cfg(target_os = "linux")]
    if !Path::new("/usr/bin/pkexec").is_file() {
        return Some(
            "Install polkit and a desktop authentication agent, then reopen this window.".into(),
        );
    }
    None
}

/// Start a user-level CLI command without a terminal or Windows console flash.
pub fn cli_command(args: &[&str]) -> Result<Command, String> {
    let mut command = unprivileged_command(&cli_path()?);
    command.args(args);
    Ok(command)
}

fn unprivileged_command(cli: &Path) -> Command {
    let mut command = Command::new(cli);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
        command.creation_flags(CREATE_NO_WINDOW.0);
    }
    command
}

pub fn perform(action: Action) -> Result<String, String> {
    let cli = cli_path()?;
    #[cfg(windows)]
    if action == Action::Upgrade && cli != sibling("hp-charge-limit-rs")? {
        return Err(
            "The bundled CLI is missing. Reinstall the desktop app to update the service.".into(),
        );
    }
    if let Some(message) = authentication_prerequisite() {
        return Err(message);
    }
    // Don't elevate an older or broken CLI even if it replaced a working copy
    // since the last refresh. The service may still run a different version.
    read_status(&cli, true)?;
    #[cfg(target_os = "linux")]
    {
        let output = elevated_command(&cli, action).output().map_err(|error| {
            format!("Could not request authentication: {error}. Check polkit and its authentication agent.")
        })?;
        let detail = [output.stdout, output.stderr]
            .into_iter()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if output.status.success() {
            Ok(detail)
        } else if output.status.code() == Some(126) {
            Err("Authentication cancelled. No changes were made.".into())
        } else if output.status.code() == Some(127) {
            Err("Authorization failed. Open the app from your desktop application menu and ensure a polkit authentication agent is running. Remote/TTY sessions may not have access to the desktop password dialog.".into())
        } else if detail.is_empty() {
            Err("Request failed or authentication was cancelled. No success was reported.".into())
        } else {
            Err(detail)
        }
    }
    #[cfg(windows)]
    {
        platform_windows::perform(&cli, action)
    }
}

#[cfg(target_os = "linux")]
fn elevated_command(cli: &Path, action: Action) -> Command {
    let mut command = Command::new("/usr/bin/pkexec");
    // A desktop app must never fall back to pkexec's terminal password agent.
    command
        .arg("--disable-internal-agent")
        .arg(cli)
        .args(action.arguments());
    command
}

#[cfg(target_os = "linux")]
fn autostart_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| "Could not locate your configuration directory".to_string())?;
    Ok(base.join("autostart/io.github.kralonur.HpChargeLimit.Tray.desktop"))
}

pub fn autostart_enabled() -> bool {
    #[cfg(target_os = "linux")]
    {
        autostart_path()
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .is_some_and(|entry| !entry.lines().any(|line| line == "Hidden=true"))
    }
    #[cfg(windows)]
    {
        platform_windows::autostart_enabled()
    }
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let path = autostart_path()?;
        if enabled {
            let tray = sibling("hp-charge-limit-tray")?;
            if !tray.is_file() {
                return Err("Install the optional tray beside the dashboard first.".into());
            }
            let entry = autostart_entry(&tray)?;
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|error| error.to_string())?;
            std::fs::write(path, entry).map_err(|error| error.to_string())
        } else {
            match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            }
        }
    }
    #[cfg(windows)]
    {
        platform_windows::set_autostart(enabled)
    }
}

#[cfg(target_os = "linux")]
fn autostart_entry(tray: &Path) -> Result<String, String> {
    let path = tray.to_str().ok_or("Tray path is not valid UTF-8")?;
    if path.contains(['\n', '\r']) {
        return Err("Tray path cannot contain newlines".into());
    }
    // Desktop Entry string escaping is applied before Exec argument unquoting.
    let quoted = path
        .replace('\\', r"\\\\")
        .replace('"', r#"\\""#)
        .replace('`', r"\\`")
        .replace('$', r"\\$")
        .replace('%', "%%");
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=HP Charge Limit Tray\nExec=\"{quoted}\"\nTerminal=false\nComment=Optional charge-limit controls; the service runs independently\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_use_only_fixed_cli_commands() {
        assert_eq!(Action::Diagnose.arguments(), ["diagnose"]);
        assert_eq!(Action::Setup(None).arguments(), ["service", "setup"]);
        for (action, verb) in [
            (Action::Start, "start"),
            (Action::Stop, "stop"),
            (Action::Startup(true), "enable"),
            (Action::Startup(false), "disable"),
            (Action::Uninstall, "uninstall"),
        ] {
            assert_eq!(action.arguments(), ["service", verb]);
        }
        assert_eq!(
            Action::SetMode(Mode::Hold).arguments(),
            ["set", "hold", "--require-idle"]
        );
        assert_eq!(Action::ReadMode.arguments(), ["firmware-status"]);
        assert_eq!(Action::Logs.arguments(), ["logs"]);
        #[cfg(windows)]
        assert_eq!(Action::Upgrade.arguments(), ["service", "upgrade"]);
        let settings = ChargeSettings {
            start: 75,
            stop: 80,
            off_mode: StopAction::Inhibit,
            advanced: Default::default(),
        };
        assert_eq!(
            Action::Configure(settings).arguments(),
            [
                "service",
                "configure",
                "--start=75",
                "--stop=80",
                "--off-mode=inhibit"
            ]
        );
        assert_eq!(Action::Bypass(true).arguments(), ["bypass", "on"]);
        assert_eq!(Action::Bypass(false).arguments(), ["bypass", "off"]);
    }

    #[test]
    fn prefers_the_bundled_cli_and_reports_a_missing_backend() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = dir.path().join("bundled-cli");
        let installed = dir.path().join("system-cli");
        assert!(
            select_cli(bundled.clone(), installed.clone())
                .unwrap_err()
                .contains("CLI not found")
        );
        std::fs::write(&installed, "").unwrap();
        assert_eq!(
            select_cli(bundled.clone(), installed.clone()).unwrap(),
            installed
        );
        std::fs::write(&bundled, "").unwrap();
        assert_eq!(select_cli(bundled.clone(), installed).unwrap(), bundled);
    }

    #[test]
    fn status_requires_a_matching_machine_readable_response() {
        let status = serde_json::json!({
            "api_version": API_VERSION,
            "battery": {"capacity": 74, "on_ac": true},
            "bypass": true,
            "service": "running",
            "removable": false
        });
        let mut status = status;
        let snapshot = decode_status(&serde_json::to_vec(&status).unwrap()).unwrap();
        assert_eq!(snapshot.battery.capacity, Some(74));
        assert_eq!(snapshot.service, ServiceStatus::Running);
        assert!(snapshot.bypass);
        status["api_version"] = serde_json::json!(API_VERSION + 1);
        assert!(decode_status(&serde_json::to_vec(&status).unwrap()).is_none());
        assert!(decode_status(b"service: active").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn setup_and_firmware_actions_require_the_module_up_front() {
        assert!(Action::Setup(None).requires_loaded_module());
        assert!(!Action::Diagnose.requires_loaded_module());
        assert!(Action::SetMode(Mode::Inhibit).requires_loaded_module());
        assert!(!Action::Start.requires_loaded_module());
        assert!(!Action::Configure(ChargeSettings::default()).requires_loaded_module());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn elevation_uses_an_executable_and_fixed_arguments_not_a_shell() {
        for action in [
            Action::Diagnose,
            Action::Setup(None),
            Action::Start,
            Action::Stop,
            Action::Startup(true),
            Action::Startup(false),
            Action::Uninstall,
            Action::SetMode(Mode::Inhibit),
            Action::ReadMode,
            Action::Logs,
            Action::LoadDriver,
            Action::Configure(ChargeSettings::default()),
            Action::Bypass(true),
            Action::Bypass(false),
        ] {
            let command = elevated_command(Path::new("/usr/bin/hp-charge-limit-rs"), action);
            assert_eq!(command.get_program(), "/usr/bin/pkexec");
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args[0], "--disable-internal-agent");
            assert_eq!(args[1], "/usr/bin/hp-charge-limit-rs");
            let expected: Vec<_> = command
                .get_args()
                .skip(2)
                .map(|arg| arg.to_str().unwrap())
                .collect();
            assert_eq!(expected, action.arguments());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn autostart_quotes_paths_and_cannot_inject_another_entry() {
        let entry = autostart_entry(Path::new("/home/me/My Apps/tray%file")).unwrap();
        assert!(entry.contains("Exec=\"/home/me/My Apps/tray%%file\"\n"));
        let entry = autostart_entry(Path::new(r#"/tmp/a"$`\b"#)).unwrap();
        assert!(entry.contains(r#"Exec="/tmp/a\\"\\$\\`\\\\b""#));
        assert!(autostart_entry(Path::new("/tmp/tray\nHidden=true")).is_err());
    }
}
