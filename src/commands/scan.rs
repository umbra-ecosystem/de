use eyre::{WrapErr, eyre};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::{
    project::Project,
    types::Slug,
    workspace::{self},
};

/// Scans the specified directory for `de.toml` files and updates the workspace configuration.
///
/// FIXME: We can improve this by only checking for config files and not all files.
pub fn scan(dir: Option<PathBuf>, workspace: Option<Slug>) -> eyre::Result<()> {
    let dir = match dir {
        Some(d) => d,
        None => std::env::current_dir()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current directory")?,
    };

    let walkdir = WalkDir::new(&dir);
    for entry in walkdir {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Error reading entry: {}", e);
                continue;
            }
        };

        if entry.file_type().is_file() {
            if entry.file_name() != "de.toml" {
                continue;
            }

            let result = update_workspace(entry.path(), workspace.as_ref());
            if let Err(e) = result {
                eprintln!(
                    "Failed to update workspace for {}: {}",
                    entry.path().display(),
                    e
                );
            } else {
                println!("Updated workspace for {}", entry.path().display());
            }
        }
    }

    Ok(())
}

fn update_workspace(manifest_path: &Path, workspace: Option<&Slug>) -> eyre::Result<()> {
    let project_path = manifest_path
        .parent()
        .ok_or_else(|| eyre!("Manifest path has no parent directory"))?;

    let project = Project::from_dir(project_path)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load project from directory")?;

    if let Some(ws) = workspace {
        if project.manifest().workspace.name != *ws {
            return Ok(());
        }
    }

    let project_id = Project::infer_name(project_path)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to infer project ID")?;

    workspace::add_project_to_workspace(
        project.manifest().workspace.name.clone(),
        project_id,
        project_path.to_path_buf(),
    )
    .wrap_err("Failed to add project to workspace")
    .map_err(|e| eyre!(e))?;

    Ok(())
}
