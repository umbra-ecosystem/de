use eyre::{Context, eyre};

use crate::{project::Project, types::Slug};

pub fn run(task_name: Slug, args: Vec<String>) -> eyre::Result<()> {
    let project = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?
        .ok_or_else(|| eyre!("No current project found"))?;

    let task = project
        .manifest()
        .tasks
        .as_ref()
        .and_then(|tasks| tasks.get(&task_name))
        .ok_or_else(|| eyre!("Task '{}' not found in project", task_name))?;

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

    Ok(())
}
