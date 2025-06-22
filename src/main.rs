mod cli;
mod commands;
mod constants;
mod project;
mod shim;
mod types;
mod utils;
mod workspace;

use clap::Parser;
use eyre::{Context, eyre};

use crate::{
    cli::{Cli, ShimCommands, TaskCommands},
    workspace::Workspace,
};

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        cli::Commands::Init { workspace } => {
            commands::init(workspace)?;
        }
        cli::Commands::Run { command, args } => {
            commands::run(command, args)?;
        }
        cli::Commands::List { workspace } => {
            if let Some(workspace_name) = workspace {
                let workspace = Workspace::load_from_name(&workspace_name)
                    .map_err(|e| eyre!(e))
                    .wrap_err("Failed to load workspace")?
                    .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;

                commands::list(workspace)?;
            } else {
                let current_workspace = workspace::Workspace::current()?
                    .ok_or_else(|| eyre!("No current workspace found"))?;
                commands::list(current_workspace)?;
            }
        }
        cli::Commands::Discover {
            directory,
            workspace,
        } => {}
        cli::Commands::Task { command } => match command {
            TaskCommands::Check { task } => {
                commands::task::check(task)?;
            }
        },
        cli::Commands::Shim { command } => match command {
            ShimCommands::Install => {
                commands::shim::install()?;
            }
            ShimCommands::Add { command } => {
                commands::shim::add(command)?;
            }
            ShimCommands::Remove { command } => {
                commands::shim::remove(command)?;
            }
            ShimCommands::List => {
                commands::shim::list()?;
            }
        },
    }

    Ok(())
}
