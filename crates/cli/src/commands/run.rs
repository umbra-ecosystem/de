use eyre::{Context, eyre};
use std::process::Command;

use crate::{
    project::{Project, TaskOrigin},
    types::Slug,
    utils::ui::UserInterface,
    workspace::Workspace,
};

pub fn run(
    task_name: Slug,
    args: Vec<String>,
    project_name: Option<Slug>,
    workspace_name: Option<Slug>,
) -> eyre::Result<()> {
    let ui = UserInterface::new();

    let workspace = match workspace_name.as_ref() {
        Some(workspace_name) => Workspace::load_from_name(workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?,
        None => Workspace::active().ok().flatten(),
    };

    if let Some(project_name) = project_name {
        let workspace = workspace.as_ref().ok_or_else(|| {
            if let Some(workspace_name) = workspace_name.as_ref() {
                eyre!("Workspace '{}' not found", workspace_name)
            } else {
                eyre!("No active workspace found")
            }
        })?;

        // If a project is specified, check if it exists in the workspace
        let ws_project = workspace
            .config()
            .projects
            .get(&project_name)
            .ok_or_else(|| {
                eyre!(
                    "Project '{}' not found in workspace '{}'",
                    project_name,
                    workspace.config().name
                )
            })?;

        let project = Project::from_dir(&ws_project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load project from directory")?;

        if !run_project_task(&project, &task_name, &args)? {
            return Err(eyre!(
                "Task '{}' not found in project '{}'",
                task_name,
                project.manifest().project().name
            ));
        }
    } else if let Some(workspace_name) = workspace_name.as_ref() {
        // If a workspace is specified but no project, check if current project is part of that workspace
        if let Some(_workspace) = &workspace {
            if let Some(project) = Project::current()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get current project")?
            {
                if &project.manifest().project().workspace == workspace_name {
                    if run_project_task(&project, &task_name, &args)? {
                        return Ok(());
                    }
                } else {
                    tracing::info!(
                        "Current project '{}' is not in workspace '{}', skipping task execution.",
                        project.manifest().project().name,
                        workspace_name
                    );
                }
            }
        } else {
            return Err(eyre!("Workspace '{}' not found", workspace_name));
        }
    } else {
        // No project or workspace specified - try current/inferred project (no-config mode)
        if let Some(project) = Project::current_or_inferred()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to detect project")?
        {
            // Show helpful indicator for inferred projects
            if project.is_inferred() {
                ui.info_item("Running task in no-config mode (no de.toml found)")?;
            }

            if run_project_task(&project, &task_name, &args)? {
                return Ok(());
            }
        }
    }

    // If project task not found, try workspace task
    let has_workspace = if let Some(ref workspace) = workspace {
        if workspace.config().tasks.contains_key(&task_name) {
            println!("Running workspace task '{task_name}'...");
            return super::workspace::run(None, task_name, args);
        }
        true
    } else {
        false
    };

    // Provide helpful error message
    ui.error_item(&format!("Task '{}' not found.", task_name), None)?;
    ui.new_line()?;
    ui.writeln("Searched in:")?;
    if has_workspace {
        ui.writeln("  • Current/inferred project tasks")?;
        ui.writeln("  • Workspace tasks")?;
    } else {
        ui.writeln("  • Configured tasks (de.toml)")?;
        ui.writeln(
            "  • Detected tasks (package.json, Makefile, justfile, Cargo.toml, pyproject.toml)",
        )?;
    }
    ui.new_line()?;
    ui.writeln(&format!(
        "Run {} to see available tasks.",
        ui.theme.highlight("de task list")
    ))?;

    std::process::exit(1)
}

pub fn run_project_task(
    project: &Project,
    task_name: &Slug,
    args: &[String],
) -> eyre::Result<bool> {
    let Some(task) = project
        .resolve_task(task_name.as_str())
        .wrap_err("Failed to resolve task")?
    else {
        return Ok(false);
    };

    if let TaskOrigin::Detected(source) = &task.origin {
        println!(
            "Running detected task '{}' from {}",
            task_name,
            source.display_name()
        );
    }

    run_to_completion(task.to_command(args))?;
    Ok(true)
}

/// Runs `command` attached to the terminal and, as a proxy, exits with its exit code on failure.
pub fn run_to_completion(mut command: Command) -> eyre::Result<()> {
    let status = command
        .status()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to execute task command")?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
