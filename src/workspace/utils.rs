use std::path::PathBuf;

use crate::{
    types::Slug,
    workspace::{Workspace, config::WorkspaceProject},
};
use eyre::{Context, eyre};

pub fn add_project_to_workspace(
    workspace_name: Slug,
    project_manifest_path: PathBuf,
) -> eyre::Result<()> {
    let mut workspace = if let Some(workspace) =
        Workspace::load_from_name(&workspace_name).map_err(|e| eyre!(e))?
    {
        workspace
    } else {
        Workspace::new(workspace_name)?
    };

    let project = WorkspaceProject::new(project_manifest_path)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load workspace project")?;

    workspace.add_project(project);

    workspace
        .save()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to save workspace configuration")?;

    Ok(())
}
