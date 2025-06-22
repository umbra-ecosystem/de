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
    for workspace_project in &workspace.config().projects {
        let project = Project::from_dir(&workspace_project.dir)?;
        let project_name = project.name().map_err(|e| eyre!(e)).wrap_err_with(|| {
            format!(
                "Failed to get project name for {}",
                workspace_project.dir.display()
            )
        })?;

        let mut message = project_name;
        if let Some(current_project) = &current_project {
            if &workspace_project.dir == current_project.dir() {
                message.push_str(" (current)");
            }
        }

        println!(" - {}", message);
    }

    Ok(())
}
