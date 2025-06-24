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

pub fn init(
    project_dir: Option<PathBuf>,
    project_name: Option<Slug>,
    workspace_name: Option<Slug>,
) -> eyre::Result<()> {
    let project_dir = if let Some(project_dir) = project_dir {
        if !project_dir.is_dir() {
            return Err(eyre!(
                "The specified project directory is not a valid directory"
            ));
        }

        project_dir
    } else {
        current_dir()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current directory")?
    };

    let project_dir = project_dir
        .canonicalize()
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to canonicalize project dir {}",
                project_dir.display()
            )
        })?;

    let workspace_name = if let Some(name) = workspace_name {
        name
    } else {
        let project = Project::from_dir(&project_dir)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load project from directory")?;
        project.manifest().workspace().name.clone()
    };

    let name = write_manifest(workspace_name.clone(), &project_dir, project_name)
        .wrap_err("Failed to write project manifest")
        .map_err(|e| eyre!(e))?;

    workspace::add_project_to_workspace(workspace_name, name, project_dir)
        .wrap_err("Failed to add project to workspace")
        .map_err(|e| eyre!(e))?;

    Ok(())
}

fn write_manifest(
    workspace_name: Slug,
    project_dir: &Path,
    project_name: Option<Slug>,
) -> eyre::Result<Slug> {
    let manifest_path = PathBuf::from("de.toml");

    let manifest = if manifest_path.exists() {
        let mut manifest = ProjectManifest::load(&manifest_path)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to load existing manifest from {}",
                    manifest_path.display()
                )
            })?;

        if let Some(name) = project_name {
            manifest.project.name = name;
        }

        manifest
    } else {
        let name = Project::infer_name(project_dir)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to infer project name from directory")?;

        ProjectManifest {
            workspace: WorkspaceManifest {
                name: workspace_name,
                ..Default::default()
            },
            project: ProjectMetadata {
                name: name.clone(),
                ..Default::default()
            },
            ..Default::default()
        }
    };

    manifest
        .save(&manifest_path)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to save manifest to {}", manifest_path.display()))?;

    Ok(manifest.project.name)
}
