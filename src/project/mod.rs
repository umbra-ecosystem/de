use std::path::PathBuf;

use eyre::{Context, eyre};

use crate::project::config::ProjectManifest;

pub mod config;

pub struct Project {
    manifest: ProjectManifest,
    manifest_path: PathBuf,
}

impl Project {
    pub fn from_manifest_path(manifest_path: PathBuf) -> eyre::Result<Self> {
        let manifest = ProjectManifest::from_file(&manifest_path)
            .map_err(|e| eyre!(e))?
            .ok_or_else(|| eyre!("Project manifest not found at {}", manifest_path.display()))?;

        Ok(Self {
            manifest,
            manifest_path,
        })
    }

    pub fn current() -> eyre::Result<Self> {
        let manifest_path = std::env::current_dir()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current working directory")?
            .join("de.toml");

        Self::from_manifest_path(manifest_path)
    }

    pub fn manifest(&self) -> &ProjectManifest {
        &self.manifest
    }

    pub fn manifest_path(&self) -> &PathBuf {
        &self.manifest_path
    }

    pub fn name(&self) -> eyre::Result<String> {
        let name = if let Some(name) = self.manifest().project().and_then(|p| p.name.as_deref()) {
            name.to_string()
        } else {
            self.manifest_path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|f| f.to_str())
                .ok_or_else(|| eyre!("Failed to extract project name from manifest path"))?
                .to_string()
        };

        Ok(name)
    }
}
