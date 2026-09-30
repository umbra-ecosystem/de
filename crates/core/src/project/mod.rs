pub mod config;
mod task;
pub mod task_detector;
pub use task::{ResolvedTask, Task, TaskOrigin, shell_command};
pub use task_detector::{DetectedTask, TaskDetectorRegistry};

use ::config::FileFormat;
use eyre::{Context, eyre};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

use crate::{
    project::config::{ProjectGitSettings, ProjectManifest, ProjectMetadata},
    types::Slug,
};

/// Default Docker Compose file names in order of precedence
/// https://docs.docker.com/compose/compose-file/
const DEFAULT_COMPOSE_FILES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

/// Check if a Docker Compose file exists in the given directory
/// Returns the path to the first matching compose file, or None if not found
fn find_compose_file_in_dir(dir: &Path) -> Option<PathBuf> {
    for filename in DEFAULT_COMPOSE_FILES {
        let path = dir.join(filename);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

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
            manifest_path: manifest_path.clone(),
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

    /// Creates a project from a directory, inferring values if no de.toml exists
    #[allow(dead_code)]
    pub fn from_dir_or_inferred(dir: &Path) -> eyre::Result<Self> {
        match Self::from_dir(dir) {
            Ok(project) => Ok(project),
            Err(_) => Self::infer_from_dir(dir),
        }
    }

    /// Creates a project with inferred values from the directory structure
    fn infer_from_dir(dir: &Path) -> eyre::Result<Self> {
        // Find the project root by searching upward for indicators
        let project_root = Self::find_inferred_project_root(dir);

        // Load .env if exists (same as from_dir)
        let dot_env = project_root.join(".env");
        if dot_env.exists() {
            dotenvy::from_path_override(&dot_env)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!(
                        "Failed to load environment variables from {}",
                        project_root.display()
                    )
                })?;
        }

        // Create inferred manifest
        let manifest = ProjectManifest {
            project: ProjectMetadata {
                name: Slug::from_dir_name(&project_root)?,
                workspace: Slug::from_str("default")
                    .map_err(|e| eyre!("Failed to create default workspace slug: {}", e))?,
                docker_compose: None, // Auto-detected by docker_compose_path()
                depends_on: None,
            },
            git: Some(ProjectGitSettings::default()),
            tasks: None,
        };

        Ok(Self {
            dir: project_root.clone(),
            manifest,
            manifest_path: project_root.join("de.toml"), // Path for reference (doesn't exist)
        })
    }

    /// Find the project root by searching upward for indicators like .git or docker-compose files
    fn find_inferred_project_root(start_dir: &Path) -> PathBuf {
        let mut current_dir = start_dir.to_path_buf();

        // First pass: search upward for .git directory (strongest indicator)
        let mut search_dir = start_dir.to_path_buf();
        loop {
            if search_dir.join(".git").is_dir() {
                return search_dir;
            }

            if !search_dir.pop() {
                break;
            }
        }

        // Second pass: search upward for docker-compose files
        loop {
            if find_compose_file_in_dir(&current_dir).is_some() {
                return current_dir;
            }

            // Try to go up one directory
            if !current_dir.pop() {
                // Reached filesystem root, return the starting directory
                return start_dir.to_path_buf();
            }
        }
    }

    /// Gets the current project, with inferred values if no de.toml exists
    pub fn current_or_inferred() -> eyre::Result<Option<Self>> {
        let current_dir = std::env::current_dir()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current working directory")?;

        // Try recursive search first
        if let Some(project) = Self::from_dir_recursive(&current_dir)? {
            return Ok(Some(project));
        }

        // Fall back to inferred from current directory
        Ok(Some(Self::infer_from_dir(&current_dir)?))
    }

    /// Checks if this project was inferred (no de.toml)
    pub fn is_inferred(&self) -> bool {
        !self.manifest_path.exists()
    }
}

impl Project {
    pub fn manifest(&self) -> &ProjectManifest {
        &self.manifest
    }

    pub fn manifest_mut(&mut self) -> &mut ProjectManifest {
        &mut self.manifest
    }

    /// Finds the task called `name`: `de.toml` first, then tasks detected from project files.
    pub fn resolve_task(&self, name: &str) -> eyre::Result<Option<ResolvedTask>> {
        if let Some(task) = self
            .manifest
            .tasks
            .as_ref()
            .and_then(|tasks| tasks.iter().find(|(key, _)| key.as_str() == name))
            .map(|(_, task)| task)
        {
            return Ok(Some(ResolvedTask {
                command: task.command_str().to_string(),
                dir: self.dir.clone(),
                origin: TaskOrigin::Configured,
            }));
        }

        Ok(self
            .detect_tasks()?
            .remove(name)
            .map(|detected| ResolvedTask {
                command: detected.command,
                dir: self.dir.clone(),
                origin: TaskOrigin::Detected(detected.source),
            }))
    }

    /// Detect tasks from project configuration files
    pub fn detect_tasks(&self) -> eyre::Result<BTreeMap<String, DetectedTask>> {
        let registry = TaskDetectorRegistry::new();
        registry.detect_all(self.dir())
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

            Ok(Some(canonical_path))
        }

        if let Some(docker_compose) = self.manifest().project().docker_compose.as_deref() {
            return canonicalize(self, docker_compose);
        }

        // Check for default Docker Compose files in order of precedence
        if let Some(compose_path) = find_compose_file_in_dir(self.dir()) {
            return canonicalize(self, &compose_path);
        }

        Ok(None)
    }

    /// Runs `docker compose -f <file> <args...>` for the project.
    ///
    /// Returns `Ok(true)` if the command succeeded, or `Ok(false)` if no Docker Compose file was found.
    pub fn compose<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> eyre::Result<bool> {
        let Some(docker_compose_path) = self
            .docker_compose_path()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get Docker Compose path")?
        else {
            return Ok(false);
        };

        let status = Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(docker_compose_path)
            .args(args)
            .current_dir(self.dir())
            .status()
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to run docker compose for project {}",
                    self.manifest().project().name
                )
            })?;

        if !status.success() {
            return Err(eyre!(
                "docker compose failed for project {} with status code: {}",
                self.manifest().project().name,
                status.code().unwrap_or(-1)
            ));
        }

        Ok(true)
    }

    /// Runs `docker compose up -d`. See [`Project::compose`].
    pub fn docker_compose_up(&self) -> eyre::Result<bool> {
        self.compose(&["up", "-d"])
    }

    /// Runs `docker compose down`. See [`Project::compose`].
    pub fn docker_compose_down(&self) -> eyre::Result<bool> {
        self.compose(&["down"])
    }
}
