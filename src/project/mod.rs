pub mod config;
mod task;
pub use task::Task;

use ::config::FileFormat;
use eyre::{Context, eyre};
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{project::config::ProjectManifest, types::Slug};

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

    pub fn from_dir_recursive(dir: &Path) -> eyre::Result<Option<Self>> {
        let mut current_dir = dir.to_path_buf();

        loop {
            if current_dir.join("de.toml").exists() {
                return Self::from_dir(&current_dir).map(Some);
            }

            if !current_dir.pop() {
                return Ok(None);
            }
        }
    }

    pub fn current() -> eyre::Result<Option<Self>> {
        let current_dir = std::env::current_dir()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current working directory")?;

        Self::from_dir_recursive(&current_dir)
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
    pub fn infer_name(dir: &Path) -> eyre::Result<Slug> {
        let dir_name = dir
            .file_name()
            .and_then(|f| f.to_str())
            .ok_or_else(|| eyre!("Failed to extract project name from manifest path"))?
            .to_string();

        let slug = Slug::sanitize(&dir_name)
            .ok_or_else(|| eyre!("Failed to sanitize project name from directory"))?;

        Ok(slug)
    }

    /// Returns the canonical path to the Docker Compose file for the project.
    pub fn docker_compose_path(&self) -> eyre::Result<Option<PathBuf>> {
        /// Canonicalizes the docker compose path, ensuring it exists and is absolute.
        fn canonicalize(project: &Project, path: &Path) -> eyre::Result<Option<PathBuf>> {
            // Check if the path is relative and resolve it against the project directory
            let path = if path.is_relative() {
                project.dir().join(path).into()
            } else {
                Cow::Borrowed(path)
            };

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

        if let Some(docker_compose) = self.manifest().project().docker_compose.as_deref() {
            return canonicalize(self, docker_compose);
        }

        let docker_compose_path = self.dir().join("docker-compose.yml");
        return canonicalize(self, &docker_compose_path);
    }

    /// Runs `docker-compose up -d` for the project, starting all services defined in the Docker Compose file.
    ///
    /// Returns `Ok(true)` if the command was successful, or `Ok(false)` if no Docker Compose file was found.
    pub fn docker_compose_up(&self) -> eyre::Result<bool> {
        let docker_compose_path = self
            .docker_compose_path()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get Docker Compose path")?;

        let Some(docker_compose_path) = docker_compose_path else {
            return Ok(false);
        };

        let status = Command::new("docker-compose")
            .arg("-f")
            .arg(docker_compose_path)
            .arg("up")
            .arg("-d")
            .status()
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to run docker-compose up for project {}",
                    self.manifest().project().name
                )
            })?;

        if !status.success() {
            return Err(eyre!(
                "docker-compose up failed with status code: {}",
                status.code().unwrap_or(-1)
            ));
        }

        Ok(true)
    }

    /// Runs `docker-compose down` for the project, stopping all services defined in the Docker Compose file.
    ///
    /// Returns `Ok(true)` if the command was successful, or `Ok(false)` if no Docker Compose file was found.
    pub fn docker_compose_down(&self) -> eyre::Result<bool> {
        let docker_compose_path = self
            .docker_compose_path()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get Docker Compose path")?;

        let Some(docker_compose_path) = docker_compose_path else {
            return Ok(false);
        };

        let status = Command::new("docker-compose")
            .arg("-f")
            .arg(docker_compose_path)
            .arg("down")
            .status()
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to run docker-compose down for project {}",
                    self.manifest().project().name
                )
            })?;

        if !status.success() {
            return Err(eyre!(
                "docker-compose down failed with status code: {}",
                status.code().unwrap_or(-1)
            ));
        }

        Ok(true)
    }
}
