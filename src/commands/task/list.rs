use eyre::{WrapErr, eyre};

use crate::{project::Project, workspace::Workspace};

pub fn list() -> eyre::Result<()> {
    let mut found_tasks = false;

    // List project tasks
    if let Some(project) = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?
    {
        if let Some(tasks) = project.manifest().tasks.as_ref() {
            if !tasks.is_empty() {
                println!("Tasks in project '{}':", project.manifest().project().name);
                for (name, task) in tasks {
                    println!("  - {}: {}", name, task.command_str());
                }
                found_tasks = true;
            }
        }
    }

    // List workspace tasks
    if let Some(workspace) = Workspace::active()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get active workspace")?
    {
        if !workspace.config().tasks.is_empty() {
            if found_tasks {
                println!(); // Add a newline for separation if project tasks were listed
            }
            println!("Tasks in workspace '{}':", workspace.config().name);
            for (name, command) in &workspace.config().tasks {
                println!("  - {}: {}", name, command);
            }
            found_tasks = true;
        }
    }

    if !found_tasks {
        println!("No tasks found in the current project or active workspace.");
    }

    Ok(())
}
