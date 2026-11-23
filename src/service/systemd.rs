//! What systemd says about the system and our service, plus fresh service
//! setup and starting an existing one.

use crate::cli::ChargeSettings;
use crate::firmware::Wmi;
use crate::paths;
use anyhow::{Context, Result, ensure};
use hp_charge_limit_protocol::ServiceStatus;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

mod settings;
pub use settings::{charge_settings, configure};

pub const SERVICE: &str = "hp-charge-limit-rs.service";
/// `is-system-running` state during shutdown and reboot. A plain
/// `systemctl stop` of one service leaves it "running"/"degraded".
const STOPPING: &str = "stopping";

const SYSTEMCTL: &str = "/usr/bin/systemctl";
const INSTALLED_BINARY: &str = "/usr/bin/hp-charge-limit-rs";
const RUNNING_IMAGE: &str = "/proc/self/exe";
const SETUP_LOCK: &str = "/run/hp-charge-limit-rs-setup.lock";
const UNIT_PATH: &str = "/etc/systemd/system/hp-charge-limit-rs.service";
const UNIT_SOURCES: &[&str] = &[
    UNIT_PATH,
    "/run/systemd/system/hp-charge-limit-rs.service",
    "/usr/lib/systemd/system/hp-charge-limit-rs.service",
    "/lib/systemd/system/hp-charge-limit-rs.service",
];
const CONFIG_PATH: &str = "/etc/hp-charge-limit-rs.conf";
const MODULES_LOAD_PATH: &str = "/etc/modules-load.d/hp-charge-limit-rs.conf";
const MODULE_SOURCES: &[&str] = &[
    "/etc/modules",
    "/etc/modules-load.d",
    "/run/modules-load.d",
    "/usr/lib/modules-load.d",
    "/lib/modules-load.d",
];
const SERVICE_UNIT: &str = include_str!("../../packaging/hp-charge-limit-rs.service");
const DEFAULT_CONFIG: &str = include_str!("../../packaging/hp-charge-limit-rs.conf");
const NOTIFY_POLICY: &str = "/etc/dbus-1/system.d/io.github.kralonur.HpChargeLimit.conf";
const PACKAGED_NOTIFY_POLICY: &str =
    "/usr/share/dbus-1/system.d/io.github.kralonur.HpChargeLimit.conf";

fn systemctl(args: &[&str]) -> Option<String> {
    let out = Command::new("systemctl").args(args).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Runs the privileged systemctl and turns a non-zero exit into an error.
fn run_systemctl(args: &[&str]) -> Result<String> {
    let out = Command::new(SYSTEMCTL)
        .args(args)
        .output()
        .with_context(|| format!("run {SYSTEMCTL}"))?;
    ensure!(
        out.status.success(),
        "{SYSTEMCTL} {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// True during shutdown/reboot; also when systemd can't be asked, since a
/// hold is the safer mistake.
pub fn system_stopping() -> bool {
    systemctl(&["is-system-running"]).is_none_or(|state| state == STOPPING)
}

pub fn service_state() -> String {
    systemctl(&["is-active", SERVICE]).unwrap_or_else(|| "?".into())
}

pub(crate) fn desktop_status() -> (ServiceStatus, Option<u32>) {
    match Command::new(SYSTEMCTL)
        .args([
            "show",
            "--no-pager",
            "--property=LoadState,ActiveState,SubState,MainPID",
            SERVICE,
        ])
        .output()
    {
        Ok(output) if !output.stdout.is_empty() => {
            parse_systemd_status(&String::from_utf8_lossy(&output.stdout))
        }
        _ => (
            ServiceStatus::Unavailable("Could not query systemd".into()),
            None,
        ),
    }
}

fn parse_systemd_status(output: &str) -> (ServiceStatus, Option<u32>) {
    let property = |key| output.lines().find_map(|line| line.strip_prefix(key));
    let state = match (property("LoadState="), property("ActiveState=")) {
        (Some("not-found"), _) => ServiceStatus::Missing,
        (Some("loaded"), Some("active")) => ServiceStatus::Running,
        (Some("loaded"), Some("activating")) => match property("SubState=") {
            Some("auto-restart" | "auto-restart-queued") => {
                ServiceStatus::Unavailable("Service exited; systemd is retrying".into())
            }
            _ => ServiceStatus::Starting,
        },
        (Some("loaded"), Some("inactive" | "failed")) => ServiceStatus::Stopped,
        _ => ServiceStatus::Unavailable("Service is transitioning, masked, or unreadable".into()),
    };
    let pid = if state == ServiceStatus::Running {
        property("MainPID=")
            .and_then(|value| value.parse().ok())
            .filter(|&pid| pid != 0)
    } else {
        None
    };
    (state, pid)
}

fn service_show() -> Result<String> {
    run_systemctl(&[
        "show",
        "--no-pager",
        "--property=LoadState,UnitFileState,FragmentPath,DropInPaths,ExecStart,EnvironmentFiles,NeedDaemonReload",
        SERVICE,
    ])
}

fn load_state(config: &str) -> Option<&str> {
    config
        .lines()
        .find_map(|line| line.strip_prefix("LoadState="))
}

/// Where the service's unit, drop-ins and environment come from, as
/// `Key=value` lines; None if systemctl failed or the unit does not exist.
pub fn service_config() -> Option<String> {
    let config = service_show().ok()?;
    (load_state(&config) != Some("not-found")).then_some(config)
}

/// Whether the unit exists, including a stopped, failed or masked one.
/// A query failure is an error, not "absent": setup must never guess.
pub fn service_exists() -> Result<bool> {
    let config = service_show().context("query systemd service state")?;
    let state = load_state(&config).unwrap_or_default();
    ensure!(!state.is_empty(), "systemctl show {SERVICE}: no LoadState");
    Ok(state != "not-found")
}

/// Start the installed service without enabling it, reloading units or
/// changing its configuration.
pub fn start() -> Result<()> {
    run_systemctl(&["start", SERVICE])
        .context("start service; run `sudo hp-charge-limit-rs service setup` if missing")?;
    println!("start requested for {SERVICE} (state: {})", service_state());
    Ok(())
}

pub fn stop() -> Result<()> {
    require_root()?;
    let _lock = setup_lock()?;
    run_systemctl(&["stop", SERVICE]).map(drop)
}

pub fn startup_enabled() -> Result<bool> {
    let unit = service_show()?;
    match unit
        .lines()
        .find_map(|line| line.strip_prefix("UnitFileState="))
    {
        Some("enabled") => Ok(true),
        Some("disabled") => Ok(false),
        _ => anyhow::bail!("This unit's startup policy is managed outside the app."),
    }
}

pub fn set_startup(enabled: bool) -> Result<()> {
    require_root()?;
    let _lock = setup_lock()?;
    startup_enabled()?;
    run_systemctl(&[if enabled { "enable" } else { "disable" }, SERVICE]).map(drop)
}

pub fn check_manual_mode() -> Result<()> {
    if service_exists()? {
        let state = run_systemctl(&["show", "--property=ActiveState", "--value", SERVICE])?;
        ensure!(
            matches!(state.as_str(), "inactive" | "failed"),
            "Stop the limiter service before setting a manual mode."
        );
    }
    Ok(())
}

pub fn recent_logs() -> Result<String> {
    let output = Command::new("/usr/bin/journalctl")
        .args(["--unit", SERVICE, "--no-pager", "--lines=20"])
        .output()
        .context("Read the system journal")?;
    ensure!(
        output.status.success(),
        "Could not read the service journal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn load_driver() -> Result<()> {
    require_root()?;
    let output = Command::new("/usr/bin/modprobe")
        .arg("acpi_call")
        .output()
        .context("Load acpi_call")?;
    ensure!(
        output.status.success() && Path::new(paths::ACPI_CALL).exists(),
        "Could not load acpi_call. Check that the module is installed for your running kernel and signed if Secure Boot is enabled. {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    println!("ACPI driver loaded.");
    Ok(())
}

pub fn can_uninstall() -> bool {
    settings::check_unit().is_ok_and(|path| path == UNIT_PATH)
        && fs::symlink_metadata(UNIT_PATH).is_ok_and(|metadata| metadata.is_file())
}

pub fn uninstall() -> Result<()> {
    require_root()?;
    let _lock = setup_lock()?;
    ensure!(
        can_uninstall(),
        "Only the standard service installed in /etc by the app can be removed here. For a packaged unit, use your package manager."
    );
    run_systemctl(&["disable", "--now", SERVICE])?;
    fs::remove_file(UNIT_PATH).context("remove service unit")?;
    run_systemctl(&["daemon-reload"])?;
    println!("Service removed. The configuration, logs and CLI were kept.");
    Ok(())
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn ensure_service_absent(exists: bool, state: &str) -> Result<()> {
    ensure!(
        !exists,
        "service {SERVICE} already exists ({state}); setup never reconfigures it; remove it first"
    );
    Ok(())
}

fn ensure_no_service() -> Result<()> {
    let exists = service_exists()?;
    ensure_service_absent(exists, &exists.then(service_state).unwrap_or_default())
}

fn ensure_unit_absent(unit: &Path) -> Result<()> {
    ensure!(
        !path_exists(unit),
        "{} already exists; setup will not overwrite it",
        unit.display()
    );
    let dropins = unit.with_extension("service.d");
    ensure!(
        !path_exists(&dropins),
        "drop-in directory {} already exists; setup will not change it",
        dropins.display()
    );
    Ok(())
}

/// An existing different CLI is never replaced automatically.
fn ensure_binary_replaceable(target: &Path, source: &Path) -> Result<()> {
    if path_exists(target) {
        ensure!(
            fs::metadata(target).is_ok_and(|m| m.is_file()),
            "{} is not a regular file; refusing to replace it",
            target.display()
        );
        ensure!(
            matches!((fs::read(target), fs::read(source)), (Ok(a), Ok(b)) if a == b),
            "{} differs; use your package manager or reinstall, then retry",
            target.display()
        );
    }
    Ok(())
}

fn module_configured(contents: &str) -> bool {
    contents.lines().any(|line| {
        let line = line.split('#').next().unwrap_or_default().trim();
        line.split_whitespace().next() == Some("acpi_call")
    })
}

fn file_configures(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|contents| module_configured(&contents))
}

fn acpi_call_configured(sources: &[&str]) -> bool {
    sources.iter().any(|source| {
        let path = Path::new(source);
        if !path.is_dir() {
            return file_configures(path);
        }
        fs::read_dir(path).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                let path = entry.path();
                path.extension()
                    .is_some_and(|ext| ext == OsStr::new("conf"))
                    && file_configures(&path)
            })
        })
    })
}

fn require_root() -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    ensure!(
        fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0),
        "service setup must run as root; use sudo"
    );
    Ok(())
}

/// Check the module and firmware before any service file is written.
fn check_firmware() -> Result<()> {
    let call = paths::at(paths::ACPI_CALL);
    ensure!(
        call.exists(),
        "{} is missing; load acpi_call with `sudo modprobe acpi_call`, then retry",
        call.display()
    );
    let mode = Wmi.read().with_context(|| {
        format!(
            "read firmware through {}; acpi_call must be loaded",
            call.display()
        )
    })?;
    println!("firmware readback: {mode}");
    Ok(())
}

#[derive(Default)]
struct SetupFiles {
    created: Vec<PathBuf>,
    enable_attempted: bool,
}

impl SetupFiles {
    fn write_new(
        &mut self,
        path: &Path,
        mode: u32,
        write: impl FnOnce(&mut File) -> io::Result<()>,
    ) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("create {} without overwriting", path.display()))?;
        // Track the file even if writing or syncing it fails.
        self.created.push(path.to_path_buf());
        write(&mut file)
            .and_then(|()| file.set_permissions(fs::Permissions::from_mode(mode)))
            .and_then(|()| file.sync_all())
            .with_context(|| format!("write {}", path.display()))
    }

    fn install_binary(&mut self, target: &Path) -> Result<()> {
        let source = Path::new(RUNNING_IMAGE);
        if path_exists(target) {
            ensure_binary_replaceable(target, source)?;
            println!("using existing {}", target.display());
            return Ok(());
        }
        let mut src = File::open(source).with_context(|| format!("open {RUNNING_IMAGE}"))?;
        self.write_new(target, 0o755, |dst| io::copy(&mut src, dst).map(drop))?;
        println!("installed {}", target.display());
        Ok(())
    }

    fn rollback(
        &self,
        unit: &Path,
        mut systemctl: impl FnMut(&[&str]) -> Result<String>,
    ) -> Result<()> {
        if self.enable_attempted {
            // Keep the files if we can't verify the service was stopped and disabled.
            systemctl(&["stop", SERVICE])?;
            systemctl(&["disable", SERVICE])?;
        }
        for path in self.created.iter().rev() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("remove {}", path.display()));
                }
            }
        }
        if self.created.iter().any(|path| path == unit) {
            systemctl(&["daemon-reload"])?;
        }
        Ok(())
    }
}

fn setup_lock() -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(SETUP_LOCK)
        .context("open service setup lock")?;
    lock.try_lock()
        .context("another service setup or settings change may be running")?;
    Ok(lock)
}

/// Create a fresh service. Refuses to touch an existing service, unit file
/// or different installed CLI.
pub fn setup(settings: Option<ChargeSettings>) -> Result<()> {
    require_root()?;
    let _lock = setup_lock()?;
    ensure_no_service()?;
    for unit in UNIT_SOURCES {
        ensure_unit_absent(Path::new(unit))?;
    }
    ensure_binary_replaceable(Path::new(INSTALLED_BINARY), Path::new(RUNNING_IMAGE))?;
    if !acpi_call_configured(MODULE_SOURCES) && path_exists(Path::new(MODULES_LOAD_PATH)) {
        ensure!(
            file_configures(Path::new(MODULES_LOAD_PATH)),
            "{MODULES_LOAD_PATH} does not load acpi_call; refusing to overwrite it"
        );
    }
    if let Some(settings) = settings {
        settings.validate()?;
        ensure!(
            !path_exists(Path::new(CONFIG_PATH)),
            "Existing configuration is kept during setup. Change the charging settings after installation."
        );
    }
    check_firmware()?;

    let mut files = SetupFiles::default();
    if let Err(error) = setup_service(&mut files, settings) {
        return match files.rollback(Path::new(UNIT_PATH), run_systemctl) {
            Ok(()) => Err(error.context("service setup failed; new files rolled back")),
            Err(cleanup) => Err(error.context(format!(
                "service setup failed and rollback was incomplete ({cleanup:#}); inspect {UNIT_PATH} and `systemctl status {SERVICE}` before retrying"
            ))),
        };
    }
    println!("enabled and started {SERVICE}");
    Ok(())
}

fn setup_service(files: &mut SetupFiles, settings: Option<ChargeSettings>) -> Result<()> {
    files.install_binary(Path::new(INSTALLED_BINARY))?;
    if settings.is_none() && path_exists(Path::new(CONFIG_PATH)) {
        println!("kept existing config {CONFIG_PATH}");
    } else {
        let config = settings.map_or_else(
            || DEFAULT_CONFIG.to_owned(),
            |settings| self::settings::with_settings(DEFAULT_CONFIG, settings),
        );
        files.write_new(Path::new(CONFIG_PATH), 0o644, |file| {
            file.write_all(config.as_bytes())
        })?;
        println!("installed config {CONFIG_PATH}");
    }
    if acpi_call_configured(MODULE_SOURCES) {
        println!("acpi_call load already configured; left module-load configuration unchanged");
    } else {
        files.write_new(Path::new(MODULES_LOAD_PATH), 0o644, |file| {
            file.write_all(b"acpi_call\n")
        })?;
        println!("configured acpi_call to load at boot in {MODULES_LOAD_PATH}");
    }

    if ![NOTIFY_POLICY, PACKAGED_NOTIFY_POLICY]
        .into_iter()
        .any(|path| path_exists(Path::new(path)))
    {
        files.write_new(Path::new(NOTIFY_POLICY), 0o644, |file| {
            file.write_all(include_bytes!(
                "../../packaging/io.github.kralonur.HpChargeLimit.conf"
            ))
        })?;
        // Some bus implementations don't reload policy automatically.
        if !Command::new("/usr/bin/busctl")
            .args([
                "--system",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "ReloadConfig",
            ])
            .output()
            .is_ok_and(|output| output.status.success())
        {
            eprintln!("Could not reload the system bus. Desktop notifications may need a reboot.");
        }
    }

    ensure_no_service()?;
    files.write_new(Path::new(UNIT_PATH), 0o644, |file| {
        file.write_all(SERVICE_UNIT.as_bytes())
    })?;
    println!("installed service unit {UNIT_PATH}");

    run_systemctl(&["daemon-reload"]).context("reload systemd units")?;
    files.enable_attempted = true;
    run_systemctl(&["enable", "--now", SERVICE]).context("enable and start the service")?;
    run_systemctl(&["is-active", "--quiet", SERVICE])
        .context("service did not reach active state")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hp-charge-limit-rs-{label}-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_file(path: &Path, contents: &[u8]) -> PathBuf {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
        path.to_path_buf()
    }

    #[test]
    fn status_only_marks_a_service_missing_when_systemd_says_so() {
        for (state, expected) in [
            (
                "LoadState=not-found\nActiveState=inactive",
                ServiceStatus::Missing,
            ),
            (
                "LoadState=loaded\nActiveState=active",
                ServiceStatus::Running,
            ),
            (
                "LoadState=loaded\nActiveState=failed",
                ServiceStatus::Stopped,
            ),
            (
                "LoadState=loaded\nActiveState=activating\nSubState=start-pre",
                ServiceStatus::Starting,
            ),
        ] {
            assert_eq!(parse_systemd_status(state), (expected, None));
        }
        for state in [
            "",
            "LoadState=masked\nActiveState=inactive",
            "LoadState=loaded\nActiveState=activating\nSubState=auto-restart",
        ] {
            assert!(matches!(
                parse_systemd_status(state),
                (ServiceStatus::Unavailable(_), None)
            ));
        }
    }

    #[test]
    fn status_reports_the_main_pid_only_for_a_running_service() {
        assert_eq!(
            parse_systemd_status("LoadState=loaded\nActiveState=active\nMainPID=123"),
            (ServiceStatus::Running, Some(123))
        );
        assert_eq!(
            parse_systemd_status("LoadState=loaded\nActiveState=inactive\nMainPID=123"),
            (ServiceStatus::Stopped, None)
        );
    }

    #[test]
    fn setup_refuses_an_existing_stopped_service() {
        let error = ensure_service_absent(true, "inactive")
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"), "{error}");
        assert!(error.contains("inactive"), "{error}");
        assert!(error.contains("never reconfigures"), "{error}");
        assert!(ensure_service_absent(false, "").is_ok());
    }

    #[test]
    fn setup_refuses_an_existing_unit_or_dropin() {
        let temp = TempDir::new("unit");
        let unit = write_file(&temp.path().join("hp-charge-limit-rs.service"), b"custom\n");
        let error = ensure_unit_absent(&unit).unwrap_err().to_string();
        assert!(error.contains("will not overwrite"), "{error}");
        assert_eq!(fs::read_to_string(&unit).unwrap(), "custom\n");
        fs::remove_file(&unit).unwrap();

        let dropins = unit.with_extension("service.d");
        fs::create_dir_all(&dropins).unwrap();
        let error = ensure_unit_absent(&unit).unwrap_err().to_string();
        assert!(error.contains("drop-in directory"), "{error}");
    }

    #[test]
    fn setup_refuses_a_different_binary_and_keeps_an_identical_one() {
        let temp = TempDir::new("binary");
        let source = write_file(&temp.path().join("source"), b"new executable");
        let target = write_file(&temp.path().join("target"), b"old executable");
        let error = ensure_binary_replaceable(&target, &source)
            .unwrap_err()
            .to_string();
        assert!(error.contains("differs"), "{error}");
        assert!(error.contains("package manager"), "{error}");

        fs::write(&target, b"new executable").unwrap();
        assert!(ensure_binary_replaceable(&target, &source).is_ok());
    }

    #[test]
    fn setup_preserves_an_existing_config() {
        let temp = TempDir::new("config");
        let config = write_file(&temp.path().join("hp-charge-limit-rs.conf"), b"custom\n");
        let mut files = SetupFiles::default();
        let defaults = |file: &mut File| file.write_all(DEFAULT_CONFIG.as_bytes());
        assert!(files.write_new(&config, 0o644, defaults).is_err());

        let missing = temp.path().join("missing.conf");
        files.write_new(&missing, 0o644, defaults).unwrap();
        assert_eq!(fs::read_to_string(&missing).unwrap(), DEFAULT_CONFIG);
        files
            .rollback(&temp.path().join("unit"), |_| unreachable!())
            .unwrap();
        assert!(!missing.exists());
        assert_eq!(fs::read_to_string(&config).unwrap(), "custom\n");
    }

    #[test]
    fn rollback_removes_even_a_partially_written_unit() {
        let temp = TempDir::new("rollback");
        let unit = temp.path().join(SERVICE);
        let mut files = SetupFiles::default();
        assert!(
            files
                .write_new(&unit, 0o644, |file| {
                    file.write_all(b"partial")?;
                    Err(io::Error::other("simulated write failure"))
                })
                .is_err()
        );
        files
            .rollback(&unit, |args| {
                assert_eq!(args, ["daemon-reload"]);
                assert!(!unit.exists());
                Ok(String::new())
            })
            .unwrap();
    }

    #[test]
    fn rollback_stops_and_disables_before_removing_the_unit() {
        let temp = TempDir::new("activation");
        let unit = temp.path().join(SERVICE);
        let mut files = SetupFiles::default();
        files.write_new(&unit, 0o644, |_| Ok(())).unwrap();
        files.enable_attempted = true;
        assert!(
            files
                .rollback(&unit, |_| Err(anyhow::anyhow!("could not stop service")))
                .is_err()
        );
        assert!(unit.exists());

        let mut calls = Vec::new();
        files
            .rollback(&unit, |args| {
                calls.push(args[0].to_string());
                assert_eq!(unit.exists(), args[0] != "daemon-reload");
                Ok(String::new())
            })
            .unwrap();
        assert_eq!(calls, ["stop", "disable", "daemon-reload"]);
    }

    #[test]
    fn module_detection_does_not_treat_comments_or_prefixes_as_configured() {
        assert!(module_configured("acpi_call\n"));
        assert!(module_configured("# load it\nacpi_call # needed\n"));
        assert!(!module_configured("# acpi_call\n"));
        assert!(!module_configured("acpi_call_extra\n"));

        let temp = TempDir::new("modules");
        let modules = write_file(&temp.path().join("modules.conf"), b"other_module\n");
        assert!(!file_configures(&modules));
        let configured = write_file(&temp.path().join("configured.conf"), b"acpi_call\n");
        assert!(file_configures(&configured));
    }
}
