use std::path::PathBuf;

use eyre::{Context, bail, eyre};

use crate::{
    config::project::{ProjectManifest, WorkspaceManifest},
    types::Slug,
};

pub fn init(workspace: Slug) -> eyre::Result<()> {
    write_manifest(workspace)
        .wrap_err("Failed to write project manifest")
        .map_err(|e| eyre!(e))?;

    Ok(())
}

fn write_manifest(workspace: Slug) -> eyre::Result<()> {
    let manifest = ProjectManifest {
        workspace: WorkspaceManifest {
            name: workspace,
            ..Default::default()
        },
        ..Default::default()
    };

    let manifest_path = PathBuf::from("de.toml");
    if manifest_path.exists() {
        bail!("Manifest file already exists");
    }

    let manifest_str = toml::to_string_pretty(&manifest)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to format manifest as string")?;

    std::fs::write(&manifest_path, manifest_str)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to write manifest to {}", manifest_path.display()))?;

    Ok(())
}
