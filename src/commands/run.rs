use eyre::{Context, eyre};

use crate::{project::Project, types::Slug, workspace::Workspace};

pub fn run(task_name: Slug, args: Vec<String>) -> eyre::Result<()> {
    let project = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?;

    if let Some(project) = project {
        if let Some(task) = project
            .manifest()
            .tasks
            .as_ref()
            .and_then(|tasks| tasks.get(&task_name))
        {
            let mut command = task
                .command(&project)
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

            return Ok(());
        }
    }

    // If project task not found, try workspace task
    if let Some(workspace) = Workspace::active()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get active workspace")?
    {
        if workspace.config().tasks.contains_key(&task_name) {
            println!("Running workspace task '{}'...", task_name);
            return super::workspace::run(None, task_name, args);
        }
    }

    Err(eyre!("Task '{}' not found in project or active workspace", task_name))
}
