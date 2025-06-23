use std::path::PathBuf;

use crate::{
    project::Project,
    types::Slug,
    workspace::{Workspace, config::WorkspaceProject},
};
use eyre::{Context, eyre};

pub fn add_project_to_workspace(workspace_name: Slug, project_dir: PathBuf) -> eyre::Result<()> {
    let mut workspace = if let Some(workspace) =
        Workspace::load_from_name(&workspace_name).map_err(|e| eyre!(e))?
    {
        workspace
    } else {
        Workspace::new(workspace_name)?
    };

    let project = WorkspaceProject::new(project_dir)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load workspace project")?;

    workspace.add_project(project);

    workspace
        .save()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to save workspace configuration")?;

    Ok(())
}

pub fn spin_up_workspace(workspace: &Workspace) -> eyre::Result<()> {
    let mut applied_projects = Vec::new();

    for wp in &workspace.config().projects {
        let project = Project::from_dir(&wp.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to load project from directory {}", wp.dir.display())
            })?;

        let project_name = project.name().map_err(|e| eyre!(e))?;

        println!("Spinning up project {}:", project_name);

        let applied = project
            .docker_compose_up()
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to spin up project {} in workspace {}",
                    project_name,
                    workspace.config().name
                )
            })?;

        if applied {
            applied_projects.push(project_name);
        }
    }

    if applied_projects.is_empty() {
        println!("- (No projects to spin up)");
    }

    Ok(())
}

pub fn spin_down_workspace(workspace: &Workspace) -> eyre::Result<()> {
    let mut applied_projects = Vec::new();

    for wp in &workspace.config().projects {
        let project = Project::from_dir(&wp.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to load project from directory {}", wp.dir.display())
            })?;

        let project_name = project.name().map_err(|e| eyre!(e))?;

        println!("Spinning down project {}:", project_name);

        let applied = project
            .docker_compose_down()
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to spin down project {} in workspace {}",
                    project_name,
                    workspace.config().name
                )
            })?;

        if applied {
            applied_projects.push(project_name);
        }
    }

    if applied_projects.is_empty() {
        println!("- (No projects to spin down)");
    }

    Ok(())
}
