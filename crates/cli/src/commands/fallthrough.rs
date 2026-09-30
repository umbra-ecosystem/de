use std::str::FromStr;

use clap::CommandFactory;
use eyre::{Context, bail, eyre};

use crate::{
    cli::Cli, commands::run::run_project_task, project::Project, types::Slug,
    utils::ui::UserInterface, workspace::Workspace,
};

#[tracing::instrument(
    skip_all,
    fields(
        command = ?args.get(0),
        args = ?args.get(1..)
    )
)]
pub fn fallthrough(args: Vec<String>) -> eyre::Result<()> {
    let ui = UserInterface::new();

    let (command, args) = split_args(args)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to parse command and arguments")?;

    // Try workspace mode first if available
    if let Ok(Some(workspace)) = Workspace::active() {
        // Check if command matches a project name in the workspace
        if let Some(ws_project) = workspace.config().projects.get(&command) {
            let project = Project::from_dir(&ws_project.dir)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to load project from directory")?;

            let (task_command, task_args) = split_args(args)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to parse command and arguments")?;

            if run_project_task(&project, &task_command, &task_args)? {
                return Ok(());
            } else {
                bail!(
                    "Task '{}' not found in project '{}'",
                    task_command,
                    project.manifest().project().name
                );
            }
        }

        // Check if current project has the task
        if let Ok(Some(project)) = Project::current() {
            if run_project_task(&project, &command, &args)? {
                return Ok(());
            }
        }
    }

    // Try current/inferred project (no-config mode)
    match Project::current_or_inferred() {
        Ok(Some(project)) => {
            if project.is_inferred() {
                ui.info_item("Trying to run task in no-config mode (no de.toml found)")?;
            }

            if run_project_task(&project, &command, &args)? {
                return Ok(());
            }

            // If we reach here, task was not found
            ui.error_item(
                &format!("Task '{}' not found in the current project.", command),
                None,
            )?;
            ui.new_line()?;
            ui.writeln("Searched in:")?;
            ui.writeln("  • Configured tasks (de.toml)")?;
            ui.writeln(
                "  • Detected tasks (package.json, Makefile, justfile, Cargo.toml, pyproject.toml)",
            )?;
            ui.new_line()?;
            ui.writeln(&format!(
                "Run {} to see available tasks.",
                ui.theme.highlight("de task list")
            ))?;

            std::process::exit(1);
        }
        Ok(None) | Err(_) => {
            ui.error_item(
                &format!(
                    "Could not determine project context for command '{}'.",
                    command
                ),
                None,
            )?;
            ui.new_line()?;
            ui.writeln("Tried:")?;
            ui.writeln("  • Active workspace projects")?;
            ui.writeln("  • Current project (de.toml)")?;
            ui.writeln("  • Inferred project (git repository or docker-compose files)")?;
            ui.new_line()?;
        }
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
