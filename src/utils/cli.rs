use eyre::{WrapErr, eyre};

use crate::{project::Project, types::Slug, workspace::Workspace};

/// Helper function to get a project based on the provided project name and workspace name.
pub fn get_project_for_cli(
    project_name: Option<Slug>,
    workspace_name: Option<Option<Slug>>,
) -> eyre::Result<Project> {
    if let Some(project_name) = project_name {
        let workspace = match workspace_name {
            Some(Some(workspace_name)) => Workspace::load_from_name(&workspace_name)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to load workspace")?
                .ok_or_else(|| eyre!("Workspace '{}' not found", workspace_name))?,
            Some(None) => Workspace::active()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get active workspace")?
                .ok_or_else(|| eyre!("No active workspace found"))?,
            None => Workspace::current()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get current workspace")?
                .ok_or_else(|| eyre!("No current workspace found"))?,
        };

        let project = workspace
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

        Project::from_dir(&project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load project from directory")
    } else {
        Project::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
            .ok_or_else(|| eyre!("No current project found"))
    }
}

/// Helper function to get a workspace based on the provided workspace name.
pub fn get_workspace_for_cli(workspace_name: Option<Option<Slug>>) -> eyre::Result<Workspace> {
    if let Some(workspace_name) = workspace_name {
        if let Some(workspace_name) = workspace_name {
            Workspace::load_from_name(&workspace_name)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to load workspace")?
                .ok_or_else(|| eyre!("Workspace '{}' not found", workspace_name))
        } else {
            Workspace::active()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get active workspace")?
                .ok_or_else(|| eyre!("No active workspace found"))
        }
    } else {
        Workspace::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current workspace")?
            .ok_or_else(|| eyre!("No current workspace found"))
    }
}
