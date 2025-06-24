use std::{
    env::current_dir,
    path::{Path, PathBuf},
};

use eyre::{Context, eyre};

use crate::{
    project::{
        Project,
        config::{ProjectManifest, ProjectMetadata, WorkspaceManifest},
    },
    types::Slug,
    workspace,
};

pub fn init(workspace_name: Slug) -> eyre::Result<()> {
    let parent_dir = current_dir()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current directory")?;

    let project_dir = parent_dir
        .canonicalize()
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to canonicalize project dir {}",
                parent_dir.display()
            )
        })?;

    let name = write_manifest(workspace_name.clone(), &project_dir)
        .wrap_err("Failed to write project manifest")
        .map_err(|e| eyre!(e))?;

    workspace::add_project_to_workspace(workspace_name, name, project_dir)
        .wrap_err("Failed to add project to workspace")
        .map_err(|e| eyre!(e))?;

    Ok(())
}

fn write_manifest(workspace: Slug, project_dir: &Path) -> eyre::Result<Slug> {
    let name = Project::infer_name(project_dir)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to infer project name from directory")?;

    let manifest = ProjectManifest {
        workspace: WorkspaceManifest {
            name: workspace,
            ..Default::default()
        },
        project: ProjectMetadata {
            name: name.clone(),
            ..Default::default()
        },
        ..Default::default()
    };

    let manifest_path = PathBuf::from("de.toml");
    if manifest_path.exists() {
        return Ok(name);
    }

    let manifest_str = toml::to_string_pretty(&manifest)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to format manifest as string")?;

    std::fs::write(&manifest_path, manifest_str)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to write manifest to {}", manifest_path.display()))?;

    Ok(name)
}
