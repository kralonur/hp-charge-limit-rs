//! The Windows service: fresh setup, install, start, uninstall and what the
//! Service Control Manager says about it. The service itself runs `daemon`
//! (see `daemon::windows`).

use crate::cli::{ChargeSettings, DaemonOpts};
use crate::firmware::Wmi;
use crate::paths;
use anyhow::{Context, Result, bail};
use hp_charge_limit_protocol::ServiceStatus;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};
use windows::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
use windows::core::{HSTRING, PCWSTR};
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceDependency, ServiceErrorControl,
    ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState,
    ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

mod settings;
pub use settings::{charge_settings, configure, upgrade};

pub const SERVICE: &str = "hp-charge-limit-rs";
const DISPLAY_NAME: &str = "HP charge limit";
const DESCRIPTION: &str =
    "Battery charge limiter using the HP BIOS WMI charge-control command (SBCC)";
/// WMI must be up before the daemon's first firmware call.
const WMI_SERVICE: &str = "Winmgmt";
const RESTART_DELAY: Duration = Duration::from_secs(5);
const FAILURE_RESET: Duration = Duration::from_secs(24 * 60 * 60);
const START_TIMEOUT: Duration = Duration::from_secs(20);
const STOP_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(250);
const IN_USE_TIMEOUT: Duration = Duration::from_secs(10);
/// ERROR_ACCESS_DENIED: what deleting a running exe gives
const ACCESS_DENIED: i32 = 5;
/// ERROR_SHARING_VIOLATION: what overwriting a running exe gives
const SHARING_VIOLATION: i32 = 32;
/// ERROR_SERVICE_DOES_NOT_EXIST
const NOT_INSTALLED: i32 = 1060;
/// ERROR_SERVICE_NOT_ACTIVE
const NOT_ACTIVE: i32 = 1062;

fn os_code(e: &windows_service::Error) -> Option<i32> {
    match e {
        windows_service::Error::Winapi(io) => io.raw_os_error(),
        _ => None,
    }
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access)
        .context("open the Service Control Manager (run as administrator)")
}

fn state_name(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Stopped => "stopped",
        ServiceState::StartPending => "starting",
        ServiceState::StopPending => "stopping",
        ServiceState::Running => "active",
        ServiceState::ContinuePending => "continuing",
        ServiceState::PausePending => "pausing",
        ServiceState::Paused => "paused",
    }
}

fn query_status() -> windows_service::Result<windows_service::service::ServiceStatus> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(SERVICE, ServiceAccess::QUERY_STATUS)?;
    service.query_status()
}

/// `active`, `stopped`, `not installed`, ... like `systemctl is-active`.
pub fn service_state() -> String {
    match query_status() {
        Ok(status) => state_name(status.current_state).into(),
        Err(e) if os_code(&e) == Some(NOT_INSTALLED) => "not installed".into(),
        Err(_) => "?".into(),
    }
}

pub(crate) fn desktop_status() -> (ServiceStatus, Option<u32>) {
    match query_status() {
        Ok(status) => match status.current_state {
            ServiceState::Running => (ServiceStatus::Running, status.process_id),
            ServiceState::StartPending => (ServiceStatus::Starting, None),
            ServiceState::Stopped => (ServiceStatus::Stopped, None),
            state => (
                ServiceStatus::Unavailable(format!("Service: {}", state_name(state))),
                None,
            ),
        },
        Err(e) if os_code(&e) == Some(NOT_INSTALLED) => (ServiceStatus::Missing, None),
        Err(_) => (
            ServiceStatus::Unavailable("Could not query the Service Control Manager".into()),
            None,
        ),
    }
}

fn classify_service_state(state: &str) -> Result<Option<&str>> {
    match state {
        "not installed" => Ok(None),
        "?" => bail!(
            "could not determine whether service {SERVICE} exists; run as administrator and check the Service Control Manager"
        ),
        _ => Ok(Some(state)),
    }
}

/// Whether the service is registered. `service_state` says "not installed"
/// only for ERROR_SERVICE_DOES_NOT_EXIST; any other failure is an error, so
/// setup never assumes a fresh service when it cannot tell.
fn existing_service() -> Result<Option<String>> {
    Ok(classify_service_state(&service_state())?.map(str::to_owned))
}

/// Command line and start type as `Key=value` lines; None if not
/// installed or unreadable.
pub fn service_config() -> Option<String> {
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    let service = manager
        .open_service(SERVICE, ServiceAccess::QUERY_CONFIG)
        .ok()?;
    let config = service.query_config().ok()?;
    Some(format!(
        "ExecStart={}\nStartType={:?}\nAccount={}",
        config.executable_path.display(),
        config.start_type,
        config
            .account_name
            .map_or("LocalSystem".into(), |a| a.to_string_lossy().into_owned()),
    ))
}

/// Where `install` puts the exe. Program Files is writable by
/// administrators only; a LocalSystem service must never run an exe that a
/// normal user (or any unelevated program) could replace.
fn install_dir() -> PathBuf {
    let base = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .unwrap_or_else(|| r"C:\Program Files".into());
    PathBuf::from(base).join(SERVICE)
}

fn installed_exe() -> PathBuf {
    install_dir().join(format!("{SERVICE}.exe"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

/// Runs `op` on the installed exe, retrying while it is in use: a stopped
/// service's process may hold its exe briefly after reporting stopped.
fn while_in_use(mut op: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let start = Instant::now();
    loop {
        match op() {
            Err(e)
                if matches!(e.raw_os_error(), Some(ACCESS_DENIED | SHARING_VIOLATION))
                    && start.elapsed() < IN_USE_TIMEOUT =>
            {
                sleep(POLL)
            }
            result => return result,
        }
    }
}

/// Copies this exe to `installed_exe` (unless it is that file); the service
/// must be stopped, since Windows locks a running exe.
fn copy_exe(fresh: bool) -> Result<PathBuf> {
    let current = std::env::current_exe().context("locate this executable")?;
    let target = installed_exe();
    if same_file(&current, &target) {
        return Ok(target);
    }
    fs::create_dir_all(install_dir()).with_context(|| install_dir().display().to_string())?;
    if fresh {
        // A copy may remain after failed setup. Reuse it only if identical;
        // create a new file with the destination directory's permissions.
        let mut source = fs::File::open(&current).context("open running CLI")?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(mut dest) => {
                if let Err(error) = io::copy(&mut source, &mut dest) {
                    drop(dest);
                    let cleanup = fs::remove_file(&target);
                    return Err(error).with_context(|| match cleanup {
                        Ok(()) => format!("copy to {}", target.display()),
                        Err(cleanup) => format!(
                            "copy to {}; could not remove the incomplete file: {cleanup}",
                            target.display()
                        ),
                    });
                }
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                anyhow::ensure!(
                    matches!((fs::read(&current), fs::read(&target)), (Ok(a), Ok(b)) if a == b),
                    "{} differs; setup will not replace it. Use `service install` to update it.",
                    target.display()
                );
                return Ok(target);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("create {}", target.display()));
            }
        }
    } else {
        while_in_use(|| fs::copy(&current, &target).map(drop))
            .with_context(|| format!("copy to {}", target.display()))?;
    }
    println!("installed {}", target.display());
    Ok(target)
}

fn info(opts: &DaemonOpts, exe: PathBuf) -> Result<ServiceInfo> {
    let mut arguments = vec![OsString::from("daemon")];
    arguments.extend(opts.to_args().into_iter().map(OsString::from));
    Ok(ServiceInfo {
        name: SERVICE.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: arguments,
        dependencies: vec![ServiceDependency::Service(WMI_SERVICE.into())],
        // LocalSystem: needed for the hpqBIntM method call
        account_name: None,
        account_password: None,
    })
}

/// Installs (or upgrades and reconfigures) the service with these daemon
/// options, set to start at boot, and (re)starts it.
pub fn install(opts: &DaemonOpts) -> Result<()> {
    install_impl(opts, false)
}

/// Install a fresh service. Refuses to touch an existing service, including
/// a stopped one.
pub fn setup(settings: Option<ChargeSettings>) -> Result<()> {
    if let Some(state) = existing_service()? {
        bail!(
            "service {SERVICE} already exists (state: {state}); `service setup` only creates a fresh service and never reconfigures or resets an existing one. Use `service install` to update it, or uninstall it first."
        );
    }
    let mut opts = DaemonOpts::default();
    if let Some(settings) = settings {
        settings.validate()?;
        opts.apply_settings(settings);
    }
    check_firmware()?;
    install_impl(&opts, true)
}

/// Start the installed service without changing its arguments or startup.
pub fn start() -> Result<()> {
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    let service =
        match manager.open_service(SERVICE, ServiceAccess::QUERY_STATUS | ServiceAccess::START) {
            Ok(service) => service,
            Err(e) if os_code(&e) == Some(NOT_INSTALLED) => {
                bail!("service {SERVICE} is not installed; run `service setup` first")
            }
            Err(e) => return Err(e).context("open service (run as administrator)"),
        };
    match service
        .query_status()
        .context("query service")?
        .current_state
    {
        ServiceState::Running => {
            println!("service {SERVICE} is already running");
            return Ok(());
        }
        ServiceState::StartPending => {
            println!("service {SERVICE} is already starting");
            return Ok(());
        }
        _ => {}
    }
    service.start::<&str>(&[]).context("start service")?;
    println!("start requested for service {SERVICE} with its existing configuration");
    Ok(())
}

pub fn stop() -> Result<()> {
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(SERVICE, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP)
        .context("open service (run as administrator)")?;
    stop_and_wait(&service)
}

pub fn startup_enabled() -> Result<bool> {
    let service = settings::open(ServiceAccess::empty())?;
    match service.query_config()?.start_type {
        ServiceStartType::AutoStart => Ok(true),
        ServiceStartType::OnDemand => Ok(false),
        ServiceStartType::Disabled => bail!(
            "The service is disabled by Windows. Enable it from Windows Services to allow manual startup."
        ),
        _ => bail!("This service uses a startup policy managed outside the app."),
    }
}

pub fn set_startup(enabled: bool) -> Result<()> {
    settings::set_startup(enabled)
}

pub fn check_manual_mode() -> Result<()> {
    anyhow::ensure!(
        existing_service()?.is_none_or(|state| state == "stopped"),
        "Stop the limiter service before setting a manual mode."
    );
    Ok(())
}

pub fn recent_logs() -> Result<String> {
    let text = fs::read_to_string(paths::at(paths::LOG)).context("Read the service log")?;
    let tail: Vec<_> = text.lines().rev().take(14).collect();
    Ok(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

pub fn can_uninstall() -> bool {
    matches!(service_state().as_str(), "active" | "stopped")
}

fn check_firmware() -> Result<()> {
    let instance = crate::firmware::interface().context(
        "find the HP BIOS WMI interface; run as administrator and make sure this is an HP laptop",
    )?;
    println!("HP BIOS WMI: {instance}");
    let mode = Wmi
        .read()
        .context("read the firmware charge mode; setup did not install or change the service")?;
    println!("firmware readback: {mode}");
    Ok(())
}

fn install_impl(opts: &DaemonOpts, fresh: bool) -> Result<()> {
    if fresh && let Some(state) = existing_service()? {
        bail!(
            "service {SERVICE} already exists (state: {state}); `service setup` only creates a fresh service and never reconfigures or resets an existing one. Use `service install` to update it, or uninstall it first."
        );
    }
    opts.validate()?;
    // fail here, interactively, rather than in the service
    let data = paths::secure_data_dir()?;
    let manager = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let mut access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::QUERY_CONFIG;
    if fresh {
        access |= ServiceAccess::DELETE;
    }
    let (service, info) = match manager.open_service(SERVICE, access) {
        // Keep this guard before the reconfigure arm: setup always passes fresh=true.
        Ok(_) if fresh => {
            bail!(
                "service {SERVICE} already exists (state: {}); `service setup` only creates a fresh service and never reconfigures or resets an existing one. Use `service install` to update it, or uninstall it first.",
                service_state()
            )
        }
        Ok(service) => {
            stop_and_wait(&service)?;
            let info = info(opts, copy_exe(false)?)?;
            service
                .change_config(&info)
                .context("update service configuration")?;
            println!("updated service {SERVICE}");
            (service, info)
        }
        Err(e) if os_code(&e) == Some(NOT_INSTALLED) => {
            let info = info(opts, copy_exe(fresh)?)?;
            let service = manager
                .create_service(&info, access)
                .context("create service")?;
            println!("created service {SERVICE}");
            (service, info)
        }
        Err(e) => return Err(e).context("open service"),
    };
    let result = configure_and_start(&service).and_then(|()| {
        if fresh {
            wait_for_start(&service)
        } else {
            Ok(())
        }
    });
    if let Err(error) = result {
        if !fresh {
            return Err(error);
        }
        // Only fresh setup reaches here with a newly created service handle.
        // Leave the protected CLI and data dir in place for a retry.
        let cleanup = stop_and_wait(&service)
            .and_then(|()| service.delete().context("delete the new service"));
        return match cleanup {
            Ok(()) => Err(error.context("service setup failed; new service registration removed")),
            Err(cleanup) => Err(error.context(format!(
                "service setup failed and rollback was incomplete ({cleanup:#}); inspect the service or run `service uninstall` before retrying"
            ))),
        };
    }
    println!(
        "{}; command: {} {}",
        if fresh { "started" } else { "start requested" },
        info.executable_path.display(),
        join(&info.launch_arguments)
    );
    println!("log and off-state record: {}", data.display());
    Ok(())
}

fn configure_and_start(service: &windows_service::service::Service) -> Result<()> {
    service
        .set_description(DESCRIPTION)
        .context("set service description")?;
    service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(FAILURE_RESET),
            reboot_msg: None,
            command: None,
            actions: Some(vec![
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: RESTART_DELAY
                };
                3
            ]),
        })
        .context("set service restart on failure")?;
    service
        .set_failure_actions_on_non_crash_failures(true)
        .context("set service restart on failure")?;
    service.start::<&str>(&[]).context("start service")
}

fn wait_for_start(service: &windows_service::service::Service) -> Result<()> {
    let start = Instant::now();
    loop {
        let state = service
            .query_status()
            .context("query new service")?
            .current_state;
        match state {
            ServiceState::Running => return Ok(()),
            ServiceState::StartPending if start.elapsed() < START_TIMEOUT => sleep(POLL),
            _ => bail!(
                "service did not reach running state (state: {})",
                state_name(state)
            ),
        }
    }
}

fn join(args: &[OsString]) -> String {
    args.iter()
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Stops the service (it applies its stop mode, auto by default) and
/// removes it.
pub fn uninstall() -> Result<()> {
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    let service = match manager.open_service(
        SERVICE,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        Ok(service) => service,
        Err(e) if os_code(&e) == Some(NOT_INSTALLED) => {
            println!("service {SERVICE} is not installed");
            return Ok(());
        }
        Err(e) => return Err(e).context("open service"),
    };
    stop_and_wait(&service)?;
    service.delete().context("delete service")?;
    println!("removed service {SERVICE}");
    remove_installed_exe();
    println!(
        "kept the log and off-state record in {}",
        paths::data_dir().display()
    );
    Ok(())
}

/// Deletes the copy `install` made; when that copy is what is running,
/// Windows deletes it at the next restart instead.
fn remove_installed_exe() {
    let (exe, dir) = (installed_exe(), install_dir());
    if !exe.exists() {
        return;
    }
    let running = std::env::current_exe().is_ok_and(|current| same_file(&current, &exe));
    if !running {
        match while_in_use(|| fs::remove_file(&exe)) {
            Ok(()) if fs::remove_dir(&dir).is_ok() => println!("removed {}", dir.display()),
            Ok(()) => println!("removed {}", exe.display()),
            Err(e) => println!("could not remove {}: {e}; delete it by hand", exe.display()),
        }
        return;
    }
    // A running exe can't be deleted but can be renamed. Moving it aside
    // first keeps the pending delete off the exe name, so a reinstall
    // before the restart keeps its new copy (and the folder, now in use).
    let aside = exe.with_extension("exe.old");
    let schedule = || -> Result<()> {
        fs::rename(&exe, &aside).context("move aside")?;
        delete_at_restart(&aside)?;
        delete_at_restart(&dir)?;
        Ok(())
    };
    match schedule() {
        Ok(()) => println!("{} will be removed at the next restart", dir.display()),
        Err(e) => println!(
            "could not remove {}: {e:#}; delete it by hand",
            dir.display()
        ),
    }
}

fn delete_at_restart(path: &Path) -> windows::core::Result<()> {
    unsafe {
        MoveFileExW(
            &HSTRING::from(path.as_os_str()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
    }
}

fn stop_and_wait(service: &windows_service::service::Service) -> Result<()> {
    match service.stop() {
        Ok(_) => {}
        Err(e) if os_code(&e) == Some(NOT_ACTIVE) => return Ok(()),
        Err(e) => return Err(e).context("stop service"),
    }
    let start = Instant::now();
    while service
        .query_status()
        .context("query service")?
        .current_state
        != ServiceState::Stopped
    {
        if start.elapsed() > STOP_TIMEOUT {
            bail!("service did not stop within {} s", STOP_TIMEOUT.as_secs());
        }
        sleep(POLL);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_service_counts_as_existing() {
        assert_eq!(classify_service_state("stopped").unwrap(), Some("stopped"));
    }

    #[test]
    fn not_installed_is_absent() {
        assert_eq!(classify_service_state("not installed").unwrap(), None);
    }

    #[test]
    fn unknown_state_is_an_error() {
        assert!(classify_service_state("?").is_err());
    }
}
