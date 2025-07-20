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
    cli::{
        Cli, Commands, GitCommands, SelfCommands, ShimCommands, TaskCommands, WorkspaceCommands,
    },
    utils::theme::Theme,
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
        Commands::Run {
            command,
            project,
            workspace,
            args,
        } => commands::run(command, args, project, workspace),
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
                let current_workspace =
                    Workspace::active()?.ok_or_else(|| eyre!("No active workspace found"))?;
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
            ShimCommands::Reinstate => commands::shim::reinstate(),
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
            WorkspaceCommands::Config {
                workspace,
                key,
                value,
                unset,
            } => commands::workspace::config(workspace, key, value, unset),
            WorkspaceCommands::Info { workspace } => commands::workspace::info(workspace),
        },
        Commands::Doctor { workspace } => commands::doctor(workspace),
        Commands::Status { workspace } => commands::status(workspace),
        Commands::Git { command } => match command {
            GitCommands::Switch {
                target_branch,
                fallback,
                on_dirty,
            } => commands::git::switch::switch(Some(target_branch), fallback, on_dirty),
            GitCommands::BaseReset {
                base_branch,
                on_dirty,
            } => commands::git::base_reset(base_branch, on_dirty),
        },
        Commands::Config { key, value, unset } => commands::config(key, value, unset),
        Commands::Fallthrough(args) => commands::fallthrough(args),
    };

    if let Err(err) = result {
        let theme = Theme::new();

        let error_prefix = theme.error("Error:");
        let cause_prefix = theme.dim("Caused by:");

        if let Some(cause) = err.source() {
            eprintln!("{error_prefix} {err}\n{cause_prefix} {cause}");
        } else {
            eprintln!("{error_prefix} {err}");
        }

        std::process::exit(1);
    }

    Ok(())
}
