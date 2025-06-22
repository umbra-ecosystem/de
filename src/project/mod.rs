pub mod config;
mod task;

use eyre::{Context, eyre};
use std::path::{Path, PathBuf};

use crate::project::config::ProjectManifest;

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

    pub fn current() -> eyre::Result<Option<Self>> {
        let manifest_path = std::env::current_dir()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current working directory")?
            .join("de.toml");

        if !manifest_path.exists() {
            return Ok(None);
        }

        let project = Self::from_manifest_path(manifest_path)?;
        Ok(Some(project))
    }

    pub fn manifest(&self) -> &ProjectManifest {
        &self.manifest
    }

    pub fn manifest_path(&self) -> &PathBuf {
        &self.manifest_path
    }
}

impl Project {
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

    /// Returns the canonical path to the Docker Compose file for the project.
    pub fn docker_compose_path(&self) -> eyre::Result<Option<PathBuf>> {
        /// Canonicalizes the docker compose path, ensuring it exists and is absolute.
        fn canonicalize(path: &Path) -> eyre::Result<Option<PathBuf>> {
            if !path.exists() {
                return Ok(None);
            }

            let canonical_path = path
                .canonicalize()
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!(
                        "Failed to canonicalize docker compose path {}",
                        path.display()
                    )
                })?;

            return Ok(Some(canonical_path));
        }

        if let Some(docker_compose) = self
            .manifest()
            .project()
            .and_then(|p| p.docker_compose.as_deref())
        {
            return canonicalize(docker_compose);
        }

        let docker_compose_path = self
            .manifest_path()
            .parent()
            .ok_or_else(|| eyre!("Failed to get parent directory of manifest path"))?
            .join("docker-compose.yml");

        return canonicalize(&docker_compose_path);
    }
}
