use super::*;
use crate::cli::{Cli, Cmd};
use anyhow::ensure;
use clap::Parser;
use std::ptr::{null, null_mut};
use windows_service::service::Service;
use windows_sys::Win32::System::Services::{ChangeServiceConfigW, SERVICE_NO_CHANGE};

/// Recognize the exact service command that `install` and `setup` build.
/// Refuse custom commands instead of guessing what their arguments mean.
fn read_command<'a>(command: &'a str, executable: &Path) -> Result<(&'a str, DaemonOpts)> {
    let custom = "The Windows service command is customized or unreadable. Dashboard editing requires a standard service installation.";
    let (image, arguments) = command.rsplit_once(" daemon ").context(custom)?;
    let path = image
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .or_else(|| (!image.contains(char::is_whitespace)).then_some(image))
        .context(custom)?;
    ensure!(Path::new(path) == executable, "{custom}");
    let arguments = arguments.split_ascii_whitespace().collect::<Vec<_>>();
    let Cmd::Daemon(opts) = Cli::try_parse_from(
        ["hp-charge-limit-rs", "daemon"]
            .into_iter()
            .chain(arguments.iter().copied()),
    )
    .map_err(|_| anyhow::anyhow!("{custom}"))?
    .cmd
    else {
        unreachable!()
    };
    // All daemon options written by the installer are explicit, so this never
    // depends on an environment variable in the desktop user's session.
    ensure!(opts.to_args() == arguments, "{custom}");
    opts.validate()?;
    Ok((image, opts))
}

pub(super) fn open(access: ServiceAccess) -> Result<Service> {
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    manager
        .open_service(SERVICE, ServiceAccess::QUERY_CONFIG | access)
        .context("Open installed service")
}

fn query_command(service: &Service) -> Result<String> {
    let config = service
        .query_config()
        .context("Read installed service configuration")?;
    ensure!(
        config.service_type == ServiceType::OWN_PROCESS,
        "Unsupported Windows service type"
    );
    config
        .executable_path
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("The Windows service command is not valid Unicode"))
}

pub fn charge_settings(installed: bool) -> Result<ChargeSettings> {
    if !installed {
        return Ok(ChargeSettings::default());
    }
    let command = query_command(&open(ServiceAccess::empty())?)?;
    let (_, opts) = read_command(&command, &installed_exe())?;
    Ok(opts.settings())
}

pub(super) fn set_startup(enabled: bool) -> Result<()> {
    use windows_sys::Win32::System::Services::{SERVICE_AUTO_START, SERVICE_DEMAND_START};
    let service = open(ServiceAccess::CHANGE_CONFIG)?;
    // An explicit enable/disable also replaces the external Disabled policy.
    change_config(
        &service,
        if enabled {
            SERVICE_AUTO_START
        } else {
            SERVICE_DEMAND_START
        },
        None,
    )
}

fn change_config(service: &Service, startup: u32, command: Option<&str>) -> Result<()> {
    let command = command.map(HSTRING::from);
    // Change only the launch command, not the boot settings, dependencies,
    // recovery options, or service account.
    let ok = unsafe {
        ChangeServiceConfigW(
            service.raw_handle(),
            SERVICE_NO_CHANGE,
            startup,
            SERVICE_NO_CHANGE,
            command.as_ref().map_or(null(), |value| value.as_ptr()),
            null(),
            null_mut(),
            null(),
            null(),
            null(),
            null(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error()).context("Save installed service settings");
    }
    Ok(())
}

fn set_command(service: &Service, command: &str) -> Result<()> {
    change_config(service, SERVICE_NO_CHANGE, Some(command))
}

pub fn configure(settings: ChargeSettings) -> Result<()> {
    settings.validate()?;
    let service = open(ServiceAccess::CHANGE_CONFIG | RESTART_ACCESS)?;
    let old = query_command(&service)?;
    let (image, mut opts) = read_command(&old, &installed_exe())?;
    opts.apply_settings(settings);
    apply_with_restart(
        &service,
        || {
            set_command(
                &service,
                &format!("{image} daemon {}", opts.to_args().join(" ")),
            )
        },
        || set_command(&service, &old),
    )
}

const RESTART_ACCESS: ServiceAccess = ServiceAccess::QUERY_STATUS
    .union(ServiceAccess::START)
    .union(ServiceAccess::STOP);

pub fn upgrade() -> Result<()> {
    let service = open(RESTART_ACCESS)?;
    let target = installed_exe();
    read_command(&query_command(&service)?, &target)?;
    let source = std::env::current_exe().context("Locate the bundled CLI")?;
    let current = fs::read(&target).context("Read the installed service executable")?;
    let replacement = fs::read(&source).context("Read the bundled service executable")?;
    if current == replacement {
        println!("Service already uses this version.");
        return Ok(());
    }
    let stage = |bytes: &[u8]| -> Result<tempfile::TempPath> {
        use std::io::Write;
        let mut file =
            tempfile::NamedTempFile::new_in(install_dir()).context("Stage service executable")?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        Ok(file.into_temp_path())
    };
    let backup = stage(&current)?;
    let replacement = stage(&replacement)?;
    let replace = |file: tempfile::TempPath| {
        // Wait until the previous service process releases its executable.
        while_in_use(|| fs::rename(&file, &target)).context("Replace the service executable")
    };
    apply_with_restart(&service, || replace(replacement), || replace(backup))
}

fn apply_with_restart(
    service: &Service,
    apply: impl FnOnce() -> Result<()>,
    restore: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let state = service
        .query_status()
        .context("Query service before saving")?
        .current_state;
    ensure!(
        matches!(state, ServiceState::Stopped | ServiceState::Running),
        "Wait for the service to finish transitioning before saving."
    );
    let start = || {
        service
            .start::<&str>(&[])
            .context("Start service")
            .and_then(|()| wait_for_start(service))
    };
    if state != ServiceState::Running {
        return apply();
    }
    stop_and_wait(service)?;
    if let Err(error) = apply() {
        return match start() {
            Ok(()) => {
                Err(error.context("Could not apply the change. The previous service is running."))
            }
            Err(restart) => Err(error.context(format!(
                "Could not restart the previous service: {restart:#}"
            ))),
        };
    }
    if let Err(error) = start() {
        let rollback = stop_and_wait(service)
            .and_then(|()| restore())
            .and_then(|()| start());
        return match rollback {
            Ok(()) => Err(error
                .context("Could not start the updated service. Previous installation restored.")),
            Err(rollback) => Err(error.context(format!(
                "Could not restart the service or complete rollback: {rollback:#}"
            ))),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::StopAction;

    #[test]
    fn reads_standard_service_command_and_keeps_other_options() {
        let executable = Path::new(r"C:\Program Files\hp-charge-limit-rs\hp-charge-limit-rs.exe");
        let command = format!(
            "\"{}\" daemon --stop=80 --start=75 --interval=15 --run-mode=hold --off-mode=inhibit --off-min=10 --stop-mode=keep",
            executable.display()
        );
        let (image, mut opts) = read_command(&command, executable).unwrap();
        assert_eq!(
            (opts.start, opts.stop, opts.off_mode),
            (75, 80, StopAction::Inhibit)
        );
        opts.start = 40;
        opts.stop = 50;
        let updated = format!("{image} daemon {}", opts.to_args().join(" "));
        assert_eq!(
            updated,
            command.replace("--stop=80 --start=75", "--stop=50 --start=40")
        );
        assert!(read_command(&command.replace(" --interval=15", ""), executable).is_err());
        assert!(read_command(&command, Path::new(r"C:\custom.exe")).is_err());
    }
}
