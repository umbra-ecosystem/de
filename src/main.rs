mod cli;
mod commands;
mod constants;
mod project;
mod types;
mod utils;
mod workspace;

use clap::Parser;
use eyre::{Context, eyre};

use crate::{cli::Cli, workspace::Workspace};

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        cli::Commands::Init { workspace } => {
            commands::init(workspace)?;
        }
        cli::Commands::Run { command } => {}
        cli::Commands::List { workspace } => {
            if let Some(workspace_name) = workspace {
                let workspace = Workspace::load_from_name(&workspace_name)
                    .map_err(|e| eyre!(e))
                    .wrap_err("Failed to load workspace")?
                    .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;

                commands::list(workspace)?;
            } else {
                let current_workspace = workspace::Workspace::current()?;
                commands::list(current_workspace)?;
            }
        }
        cli::Commands::Discover {
            directory,
            workspace,
        } => {}
    }

    Ok(())
}
