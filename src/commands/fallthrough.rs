use std::str::FromStr;

use clap::CommandFactory;
use eyre::{Context, bail, eyre};

use crate::{
    cli::Cli, commands::run::run_project_task, project::Project, types::Slug, utils::theme::Theme,
    workspace::Workspace,
};

pub fn fallthrough(args: Vec<String>) -> eyre::Result<()> {
    let workspace = Workspace::active()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get active workspace")?
        .ok_or_else(|| eyre!("No current workspace found"))?;

    let (command, args) = split_args(args)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to parse command and arguments")?;

    if let Some(ws_project) = workspace.config().projects.get(&command) {
        let project = Project::from_dir(&ws_project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load project from directory")?;

        let (command, args) = split_args(args)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to parse command and arguments")?;

        if run_project_task(&project, &command, &args)? {
            return Ok(());
        } else {
            bail!(
                "Task '{}' not found in project '{}'",
                command,
                project.manifest().project().name
            );
        }
    }

    if let Some(project) = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?
    {
        if run_project_task(&project, &command, &args)? {
            return Ok(());
        }
    }

    {
        let theme = Theme::new();
        let error_prefix = theme.error("Error:");
        eprintln!(
            "{error_prefix} Project or task not found for '{command}' in the current context."
        );
        eprintln!();
    }

    Cli::command()
        .print_help()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to print help")?;

    Ok(())
}

fn split_args(args: Vec<String>) -> eyre::Result<(Slug, Vec<String>)> {
    let mut parts = args.into_iter();

    let command = match parts.next().as_deref().map(Slug::from_str).transpose() {
        Ok(Some(command)) => command,
        Ok(None) => {
            Cli::command().print_help()?;
            std::process::exit(1);
        }
        Err(e) => {
            return Err(eyre!("Invalid command: {}", e));
        }
    };

    let args = parts.collect::<Vec<_>>();

    Ok((command, args))
}
