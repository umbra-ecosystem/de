mod cli;
mod commands;
mod config;
mod constants;
mod project;
mod types;
mod utils;
mod workspace;

use clap::Parser;
use eyre::{Context, eyre};

use crate::{
    cli::{Cli, Commands, SelfCommands, ShimCommands, TaskCommands, WorkspaceCommands},
    workspace::Workspace,
};

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init {
            path,
            name,
            workspace,
        } => commands::init(path, name, workspace),
        Commands::Start { workspace } => commands::start(workspace),
        Commands::Stop { workspace } => commands::stop(workspace),
        Commands::Run { command, args } => commands::run(command, args),
        Commands::Exec {
            project,
            workspace,
            command,
        } => commands::exec(project, workspace, command),
        Commands::ExecAll { workspace, command } => commands::exec_all(workspace, command),
        Commands::List { workspace } => {
            if let Some(workspace_name) = workspace {
                let workspace = Workspace::load_from_name(&workspace_name)
                    .map_err(|e| eyre!(e))
                    .wrap_err("Failed to load workspace")?
                    .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;

                commands::list(workspace)
            } else {
                let current_workspace = workspace::Workspace::current()?
                    .ok_or_else(|| eyre!("No current workspace found"))?;
                commands::list(current_workspace)
            }
        }
        Commands::Scan { dir, workspace } => commands::scan(dir, workspace),
        Commands::Update { all, workspace } => commands::update(all, workspace),
        Commands::Task { command } => match command {
            TaskCommands::Check { task } => commands::task::check(task),
            TaskCommands::List => commands::task::list(),
            TaskCommands::Add {
                task,
                task_command,
                service,
                project,
                workspace,
            } => commands::task::add(task, task_command, service, project, workspace),
            TaskCommands::Remove {
                task,
                project,
                workspace,
            } => commands::task::remove(task, project, workspace),
        },
        #[cfg(target_family = "unix")]
        Commands::Shim { command } => match command {
            ShimCommands::Add { command } => commands::shim::add(command),
            ShimCommands::Remove { command } => commands::shim::remove(command),
            ShimCommands::List => commands::shim::list(),
            ShimCommands::Install => commands::shim::install(),
            ShimCommands::Uninstall => commands::shim::uninstall(),
        },
        Commands::Self_ { command } => match command {
            SelfCommands::Update => commands::self_::update(),
        },
        Commands::Workspace { command } => match command {
            WorkspaceCommands::Run {
                task,
                workspace,
                args,
            } => commands::workspace::run(workspace, task, args),
        },
        Commands::Doctor { workspace } => commands::doctor(workspace),
        Commands::Status { workspace } => commands::status(workspace),
        Commands::Fallthrough(args) => commands::fallthrough(args),
    };

    if let Err(err) = result {
        if let Some(cause) = err.source() {
            eprintln!("Error: {err}\n\nCause: {cause}");
        } else {
            eprintln!("Error: {err}");
        }
        std::process::exit(1);
    }

    Ok(())
}
