mod cli;
mod commands;
mod utils;

// Core modules are used throughout the CLI under their original paths.
use de_core::{config, constants, project, types, workspace};

use clap::Parser;
use eyre::{Context, eyre};
use tracing_subscriber::EnvFilter;

use crate::{
    cli::{
        Cli, Commands, GitCommands, SelfCommands, TaskCommands, TicketCommands, WorkspaceCommands,
    },
    utils::theme::Theme,
    workspace::Workspace,
};

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init()
        .expect("Failed to initialize tracing subscriber");

    color_eyre::config::HookBuilder::default()
        .display_env_section(false)
        .install()?;

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init {
            path,
            name,
            workspace,
        } => commands::init(path, name, workspace),
        Commands::Start { workspace, yes } => commands::start(workspace, yes),
        Commands::Stop { workspace, yes } => commands::stop(workspace, yes),
        Commands::Git { command } => match command {
            GitCommands::Status { workspace } => commands::git::status(workspace),
        },
        Commands::Ticket { command } => match command {
            TicketCommands::Claim { key, title, hotfix } => {
                commands::ticket::claim(key, title, hotfix)
            }
            TicketCommands::List => commands::ticket::list(),
            TicketCommands::Show { key, workspace } => commands::ticket::show(key, workspace),
            TicketCommands::Link {
                key,
                repo,
                branch,
                exclude,
                workspace,
            } => commands::ticket::link(key, repo, branch, exclude, workspace),
            TicketCommands::Activate {
                key,
                baseline,
                fetch,
                workspace,
            } => commands::ticket::activate_cmd(key, baseline, fetch, workspace),
            TicketCommands::Park => commands::ticket::deactivate_cmd(None),
            TicketCommands::Deactivate { status } => commands::ticket::deactivate_cmd(status),
            TicketCommands::Note { key, text } => commands::ticket::note(key, text),
            TicketCommands::Check { command } => commands::ticket::check(command),
        },
        Commands::Compose {
            project,
            workspace,
            args,
        } => commands::compose(project, workspace, args),
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
            TaskCommands::List => commands::task::list(),
            TaskCommands::Add {
                task,
                task_command,
                project,
                workspace,
            } => commands::task::add(task, task_command, project, workspace),
            TaskCommands::Remove {
                task,
                project,
                workspace,
            } => commands::task::remove(task, project, workspace),
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
        Commands::Config { key, value, unset } => commands::config(key, value, unset),
        Commands::Fallthrough(args) => commands::fallthrough(args),
    };

    if let Err(err) = result {
        let theme = Theme::new();

        let error_prefix = theme.error("Error:");
        let cause_prefix = theme.dim("Caused by:");

        eprintln!("{error_prefix} {err}");
        for cause in err.chain().skip(1) {
            eprintln!("{cause_prefix} {cause}");
        }

        std::process::exit(1);
    }

    Ok(())
}
