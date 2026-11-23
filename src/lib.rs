//! Battery charge limiter using the firmware's own
//! ACPI WMI battery-charge-control method `\_SB.WMID.SBCC`: via acpi_call
//! on Linux, via the HP BIOS WMI class `hpqBIntM` on Windows.
//!
//! Firmware modes (see `firmware`):
//!   auto     normal charging
//!   inhibit  SBIS: no charging, white LED; cleared by the EC at shutdown
//!   hold     SBTC: no charging, survives shutdown (ec-re/SBTC_HOLD.md),
//!            but LED and sysfs status say "Charging"
//! Only these three are ever written: never discharge or battery ship mode.

#[cfg(windows)]
mod acl;
pub mod bypass;
pub mod cli;
mod daemon;
mod diagnose;
mod events;
mod firmware;
#[cfg(target_os = "linux")]
mod notify;
pub mod offstate;
mod paths;
pub mod power;
pub mod service;
pub mod status;
#[cfg(windows)]
mod wmi;

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("hp-charge-limit-rs supports Linux and Windows only");

use anyhow::{Context, Result, ensure};
use clap::CommandFactory;
use cli::{Cli, Cmd};
use firmware::{ChargeControl, Wmi};

pub fn run(cli: Cli) -> Result<()> {
    #[cfg(windows)]
    let show_report = |text: &str| {
        if cli.desktop_report {
            desktop_message(text);
        } else {
            println!("{text}");
        }
    };
    #[cfg(not(windows))]
    let show_report = |text: &str| println!("{text}");
    match cli.cmd {
        Cmd::Status { watch, json, brief } => match (json, watch) {
            (true, Some(seconds)) => status::write_watch_json(seconds),
            (true, None) => status::write_json(brief),
            (false, _) => status::show(watch),
        },
        Cmd::Diagnose => {
            let (report, ok) = diagnose::run();
            show_report(report.trim_end());
            ensure!(ok, "Diagnostics found problems; see the report.");
            Ok(())
        }
        Cmd::FirmwareStatus => {
            show_report(&format!("Firmware reports {}", Wmi.read()?));
            Ok(())
        }
        Cmd::Logs => {
            show_report(&service::recent_logs()?);
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Cmd::LoadDriver => service::load_driver(),
        #[cfg(target_os = "linux")]
        Cmd::Notify => zbus::block_on(notify::run()),
        Cmd::Set { mode, require_idle } => {
            if require_idle {
                service::check_manual_mode()?;
            }
            let mut wmi = Wmi;
            wmi.set(mode)?;
            show_report(&format!(
                "Set {mode} → firmware reports {}",
                wmi.read()
                    .context("The mode was requested, but firmware readback failed")?
            ));
            Ok(())
        }
        Cmd::Bypass { action } => bypass::set(action),
        Cmd::Daemon(opts) => daemon::run(&opts),
        Cmd::Service { action } => match action {
            cli::ServiceAction::Setup { settings } => service::setup(settings.settings()),
            cli::ServiceAction::Configure { settings } => service::configure(
                settings
                    .settings()
                    .context("Provide --start, --stop and --off-mode together")?,
            ),
            cli::ServiceAction::Start => service::start(),
            cli::ServiceAction::Stop => service::stop(),
            cli::ServiceAction::Enable => service::set_startup(true),
            cli::ServiceAction::Disable => service::set_startup(false),
            #[cfg(windows)]
            cli::ServiceAction::Install(opts) => service::install(&opts),
            #[cfg(windows)]
            cli::ServiceAction::Upgrade => service::upgrade(),
            cli::ServiceAction::Uninstall => service::uninstall(),
        },
        Cmd::Completions { shell } => {
            use std::io::Write;
            std::io::stdout().write_all(completions(shell).as_bytes())?;
            Ok(())
        }
    }
}

pub fn completions(shell: cli::Shell) -> String {
    let mut script = Vec::new();
    clap_complete::generate(
        shell,
        &mut Cli::command(),
        "hp-charge-limit-rs",
        &mut script,
    );
    String::from_utf8(script).expect("clap emits UTF-8 completion scripts")
}

#[cfg(windows)]
pub fn desktop_message(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_OK, MB_SETFOREGROUND, MessageBoxW};
    use windows::core::{HSTRING, w};
    let text = HSTRING::from(text.replace('\0', "�"));
    unsafe {
        MessageBoxW(None, &text, w!("HP Charge Limit"), MB_OK | MB_SETFOREGROUND);
    }
}
