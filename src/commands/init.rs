use std::{env::current_dir, path::PathBuf};

use eyre::{Context, eyre};

use crate::{
    project::config::{ProjectManifest, WorkspaceManifest},
    types::Slug,
    workspace,
};

pub fn init(workspace_name: Slug) -> eyre::Result<()> {
    let manifest_path = write_manifest(workspace_name.clone())
        .wrap_err("Failed to write project manifest")
        .map_err(|e| eyre!(e))?;

    let parent_dir = current_dir()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current directory")?;

    // Ensure the manifest path is absolute and canonicalized
    let parent_dir = parent_dir
        .canonicalize()
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to canonicalize manifest path {}",
                manifest_path.display()
            )
        })?;

    workspace::add_project_to_workspace(workspace_name, parent_dir)
        .wrap_err("Failed to add project to workspace")
        .map_err(|e| eyre!(e))?;

    Ok(())
}

fn write_manifest(workspace: Slug) -> eyre::Result<PathBuf> {
    let manifest = ProjectManifest {
        workspace: WorkspaceManifest {
            name: workspace,
            ..Default::default()
        },
        ..Default::default()
    };

    let manifest_path = PathBuf::from("de.toml");
    if manifest_path.exists() {
        return Ok(manifest_path);
    }

    let manifest_str = toml::to_string_pretty(&manifest)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to format manifest as string")?;

    std::fs::write(&manifest_path, manifest_str)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to write manifest to {}", manifest_path.display()))?;

    Ok(manifest_path)
}
