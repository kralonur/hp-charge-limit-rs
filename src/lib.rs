//! Battery charge limiter using the firmware's own
//! ACPI WMI battery-charge-control method `\_SB.WMID.SBCC` via acpi_call.
//!
//! Firmware modes (see `firmware`):
//!   auto     normal charging
//!   inhibit  SBIS: no charging, white LED; cleared by the EC at shutdown
//!   hold     SBTC: no charging, survives shutdown (ec-re/SBTC_HOLD.md),
//!            but LED and sysfs status say "Charging"
//! Only these three are ever written: never discharge or battery ship mode.

pub mod cli;
mod bypass;
mod daemon;
mod firmware;
mod offstate;
mod paths;
mod power;
mod status;
mod systemd;

use anyhow::Result;
use cli::{Cli, Cmd};
use firmware::{ChargeControl, Wmi};

pub fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Status { watch } => status::show(watch),
        Cmd::Set { mode } => {
            let mut wmi = Wmi;
            wmi.set(mode)?;
            println!("set {mode} -> firmware reports {}", wmi.read()?);
            Ok(())
        }
        Cmd::Bypass { action } => bypass::set(action),
        Cmd::Daemon(opts) => daemon::run(&opts),
    }
}
