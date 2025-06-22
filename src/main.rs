mod cli;
mod commands;
mod constants;
mod manifest;
mod types;
mod utils;
mod workspace;

use clap::Parser;

use crate::cli::Cli;

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        cli::Commands::Init { workspace } => {
            commands::init(workspace)?;
        }
        cli::Commands::Run { command } => {}
        cli::Commands::List { workspace } => {}
        cli::Commands::Discover {
            directory,
            workspace,
        } => {}
    }

    Ok(())
}
