use eyre::{Context, eyre};

use crate::{project::Project, types::Slug, workspace::Workspace};

pub fn run(
    task_name: Slug,
    args: Vec<String>,
    project_name: Option<Slug>,
    workspace_name: Option<Slug>,
) -> eyre::Result<()> {
    let workspace = match workspace_name.as_ref() {
        Some(workspace_name) => Workspace::load_from_name(workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?,
        None => Workspace::active()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get active workspace")?,
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
        // If a workspace is specified, check if the current project is part of that workspace
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
        // If no project specified, try to run the task in the current project
        if let Some(project) = Project::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
        {
            if run_project_task(&project, &task_name, &args)? {
                return Ok(());
            }
        }
    }

    // If project task not found, try workspace task
    if let Some(workspace) = workspace {
        if workspace.config().tasks.contains_key(&task_name) {
            println!("Running workspace task '{task_name}'...");
            return super::workspace::run(None, task_name, args);
        }
    }

    Err(eyre!(
        "Task '{}' not found in project or active workspace",
        task_name
    ))
}

pub fn run_project_task(
    project: &Project,
    task_name: &Slug,
    args: &Vec<String>,
) -> eyre::Result<bool> {
    if let Some(task) = project
        .manifest()
        .tasks
        .as_ref()
        .and_then(|tasks| tasks.get(task_name))
    {
        let mut command = task
            .command(project)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to build command for task")?;

        if !args.is_empty() {
            command.args(args);
        }

        let status = command
            .status()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to execute task command")?;

        if !status.success() {
            return Err(eyre!("Task '{}' failed with status: {}", task_name, status));
        }

        return Ok(true);
    }

    Ok(false)
}
