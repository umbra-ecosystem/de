use eyre::{WrapErr, eyre};

use crate::project::Project;

pub fn list() -> eyre::Result<()> {
    let project = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?
        .ok_or_else(|| eyre!("No current project found"))?;

    let Some(tasks) = project.manifest().tasks.as_ref() else {
        println!("No tasks found in the current project.");
        return Ok(());
    };

    println!("Tasks in project '{}':", project.manifest().project().name);
    for (name, task) in tasks {
        println!(" - {}: {}", name, task.command_str());
    }

    Ok(())
}
