use eyre::{WrapErr, eyre};

use crate::{project::Project, utils::theme::Theme, workspace::Workspace};

pub fn list() -> eyre::Result<()> {
    let mut found_tasks = false;
    let theme = Theme::new();

    // List project tasks
    if let Some(project) = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?
    {
        if let Some(tasks) = project.manifest().tasks.as_ref() {
            if !tasks.is_empty() {
                println!(
                    "{} {}",
                    theme.bold("Tasks in project:"),
                    theme.highlight(project.manifest().project().name.as_str())
                );
                for (name, task) in tasks {
                    println!("- {}: {}", name.as_str(), theme.dim(&task.command_str()));
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
                println!("  - {}: {}", theme.accent(name.as_str()), command);
            }
            found_tasks = true;
        }
    }

    if !found_tasks {
        println!("No tasks found in the current project or active workspace.");
    }

    Ok(())
}
