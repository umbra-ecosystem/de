use eyre::{Context, eyre};

use crate::{project::Project, workspace::Workspace};

pub fn list(workspace: Workspace) -> eyre::Result<()> {
    let name = &workspace.config().name;

    if workspace.config().projects.is_empty() {
        println!("No projects found in workspace '{}'", name);
        return Ok(());
    }

    let current_project = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?;

    println!("Projects in workspace '{}':", name);
    for (id, wp) in &workspace.config().projects {
        let mut message = id.to_string();
        if let Some(current_project) = &current_project {
            if &wp.dir == current_project.dir() {
                message.push_str(" (current)");
            }
        }

        println!(" - {}", message);
    }

    Ok(())
}
