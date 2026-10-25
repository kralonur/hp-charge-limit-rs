use clap::Parser;

fn main() -> anyhow::Result<()> {
    hp_charge_limit_rs::run(hp_charge_limit_rs::cli::Cli::parse())
}
