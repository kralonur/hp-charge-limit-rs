use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = hp_charge_limit_rs::cli::Cli::parse();
    #[cfg(windows)]
    let show_error =
        cli.desktop_report && !matches!(cli.cmd, hp_charge_limit_rs::cli::Cmd::Diagnose);
    let result = hp_charge_limit_rs::run(cli);
    #[cfg(windows)]
    if show_error && let Err(error) = &result {
        hp_charge_limit_rs::desktop_message(&format!("{error:#}"));
    }
    result
}
