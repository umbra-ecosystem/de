pub mod config;
mod task;

use ::config::FileFormat;
use eyre::{Context, eyre};
use std::path::{Path, PathBuf};

use crate::project::config::ProjectManifest;

pub struct Project {
    dir: PathBuf,
    manifest: ProjectManifest,
    manifest_path: PathBuf,
}

impl Project {
    pub fn from_dir(dir: &Path) -> eyre::Result<Self> {
        use ::config;

        let dot_env = dir.join(".env");
        if dot_env.exists() {
            dotenvy::from_path_override(&dot_env)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!(
                        "Failed to load environment variables from {}",
                        dir.display()
                    )
                })?;
        }

        let manifest_path = dir
            .join("de.toml")
            .canonicalize()
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to canonicalize directory {}", dir.display()))?;

        let manifest_path_str = manifest_path
            .to_str()
            .map(|s| s.to_string())
            .ok_or_else(|| eyre!("Failed to convert directory path to string"))?;

        let dot_manifest_path = dir
            .join(".de/config.toml")
            .to_str()
            .map(|s| s.to_string())
            .ok_or_else(|| eyre!("Failed to convert hidden config path to string"))?;

        let builder = config::Config::builder()
            .add_source(config::File::new(
                manifest_path_str.as_str(),
                FileFormat::Toml,
            ))
            .add_source(
                config::File::new(dot_manifest_path.as_str(), FileFormat::Toml).required(false),
            )
            .add_source(config::Environment::with_prefix("DE").separator("_"))
            .build()
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to load project manifest from {}", dir.display()))?;

        let manifest = builder
            .try_deserialize::<ProjectManifest>()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to deserialize project manifest")?;

        Ok(Self {
            manifest,
            manifest_path: manifest_path.to_path_buf(),
            dir: dir.to_path_buf(),
        })
    }

    pub fn current() -> eyre::Result<Option<Self>> {
        let current_dir = std::env::current_dir()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current working directory")?;

        let manifest_path = current_dir.join("de.toml");

        if !manifest_path.exists() {
            return Ok(None);
        }

        let project = Self::from_dir(&current_dir)?;
        Ok(Some(project))
    }
}

impl Project {
    pub fn manifest(&self) -> &ProjectManifest {
        &self.manifest
    }

    pub fn manifest_path(&self) -> &PathBuf {
        &self.manifest_path
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
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
