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
    cli::{Cli, Commands, ShimCommands, TaskCommands},
    workspace::Workspace,
};

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { workspace } => {
            commands::init(workspace)?;
        }
        Commands::Run { command, args } => {
            commands::run(command, args)?;
        }
        Commands::List { workspace } => {
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
        Commands::Scan {
            directory,
            workspace,
        } => {}
        Commands::Task { command } => match command {
            TaskCommands::Check { task } => {
                commands::task::check(task)?;
            }
            TaskCommands::List => {
                commands::task::list()?;
            }
        },
        Commands::Shim { command } => match command {
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
