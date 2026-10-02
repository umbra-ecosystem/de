use eyre::{WrapErr, eyre};

use crate::{project::Project, types::Slug, workspace::Workspace};

/// Helper function to get a project based on the provided project name and workspace name.
pub fn get_project_for_cli(
    project_name: Option<Slug>,
    workspace_name: Option<Option<Slug>>,
) -> eyre::Result<Project> {
    if let Some(project_name) = project_name {
        let workspace = match workspace_name {
            Some(Some(workspace_name)) => {
                // The registry's wording, as everywhere else: a wrong or broken file says
                // which file to fix or which command to run next.
                crate::workspace::registry::load_workspace(&workspace_name).map_err(|e| eyre!(e))?
            }
            Some(None) => Workspace::active()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get active workspace")?
                .ok_or_else(no_workspace_open)?,
            None => Workspace::current()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get current workspace")?
                .ok_or_else(no_workspace_open)?,
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
            // The registry's wording, so a wrong or broken file says the same thing here as
            // it does in `de workspace select`.
            Ok(crate::workspace::registry::load_workspace(&workspace_name)
                .map_err(|e| eyre!(e))?)
        } else {
            Workspace::active()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get active workspace")?
                .ok_or_else(no_workspace_open)
        }
    } else {
        Workspace::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current workspace")?
            .ok_or_else(no_workspace_open)
    }
}

/// What to do when there is no workspace to work in.
pub(crate) fn no_workspace_open() -> eyre::Report {
    eyre!(
        "No workspace is open. Run `de workspace select <name>` to open one; \
         `de workspace list` shows the saved ones."
    )
}
