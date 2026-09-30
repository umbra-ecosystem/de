use std::{
    env::current_dir,
    path::{Path, PathBuf},
    str::FromStr,
};

use eyre::{Context, eyre};

use crate::{
    project::{
        Project,
        config::{ProjectManifest, ProjectMetadata},
    },
    types::Slug,
    workspace::{self, Workspace},
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

    println!("Initializing project in {}", project_dir.display());

    let workspace_name = if let Some(name) = workspace_name {
        name
    } else {
        prompt_workspace_name()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to prompt for workspace name")?
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
        } else {
            manifest.project.name = prompt_project_name(project_dir)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to prompt for project name")?;
        }

        manifest
    } else {
        let name = prompt_project_name(project_dir)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to prompt for project name")?;

        ProjectManifest {
            project: ProjectMetadata {
                name: name.clone(),
                workspace: workspace_name,
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

fn prompt_workspace_name() -> eyre::Result<Slug> {
    use dialoguer::Input;

    let default_name = Workspace::active()
        .ok()
        .flatten()
        .map(|ws| ws.config().name.to_string());

    let mut prompt = Input::new().with_prompt("Enter workspace name");

    if let Some(name) = default_name {
        prompt = prompt.default(name);
    }

    let name = prompt
        .validate_with(|input: &String| {
            Slug::from_str(input)
                .map(|_| ())
                .map_err(|_| "Invalid workspace name. Must be a valid slug.")
        })
        .interact_text()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to read workspace name")?;

    Slug::from_str(&name)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to parse workspace name")
}

fn prompt_project_name(project_dir: &Path) -> eyre::Result<Slug> {
    use dialoguer::Input;

    let default_name = Project::infer_name(project_dir).ok();

    let mut prompt = Input::new().with_prompt("Enter project name");

    if let Some(name) = default_name {
        prompt = prompt.default(name.to_string());
    }

    let name = prompt
        .validate_with(|input: &String| {
            Slug::from_str(input)
                .map(|_| ())
                .map_err(|_| "Invalid project name. Must be a valid slug.")
        })
        .interact_text()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to read project name")?;

    Slug::from_str(&name)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to parse project name")
}
