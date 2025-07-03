use eyre::{Context, eyre};

use crate::types::Slug;

pub fn check(task: Slug) -> eyre::Result<()> {
    let current_project = crate::project::Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?
        .ok_or_else(|| eyre!("No current project found"))?;

    current_project
        .manifest()
        .tasks
        .as_ref()
        .and_then(|tasks| tasks.get(&task))
        .ok_or_else(|| eyre!("Task '{}' not found in project", task))?;

    println!("Task '{task}' exists in the current project.");

    Ok(())
}
