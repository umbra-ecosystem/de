mod config;
mod utils;

use eyre::{Context, eyre};
use std::path::{Path, PathBuf};

use crate::{
    project::Project,
    types::Slug,
    utils::get_project_dirs,
    workspace::config::{WorkspaceConfig, WorkspaceProject},
};

pub use utils::add_project_to_workspace;

#[derive(Debug)]
pub struct Workspace {
    config: WorkspaceConfig,
    config_path: PathBuf,
}

impl Workspace {
    pub fn new(name: Slug) -> eyre::Result<Self> {
        let config_path = Self::path_from_name(&name)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to create path for workspace {}", name))?;

        let config = WorkspaceConfig {
            name,
            projects: Vec::new(),
        };

        Ok(Self {
            config_path,
            config,
        })
    }

    pub fn get_project(&self, manifest_path: &Path) -> Option<&WorkspaceProject> {
        self.config
            .projects
            .iter()
            .find(|p| p.manifest == manifest_path)
    }

    pub fn add_project(&mut self, project: WorkspaceProject) {
        self.config.projects.push(project);
        self.dedup_projects();
    }

    /// Deduplicate and sort projects in the workspace.
    pub fn dedup_projects(&mut self) {
        self.config.projects.sort_by_key(|p| p.manifest.clone());
        self.config.projects.dedup_by_key(|p| p.manifest.clone());
    }

    pub fn load_from_name(name: &Slug) -> eyre::Result<Option<Self>> {
        let workspace_config_path = Self::path_from_name(name)?;

        if !workspace_config_path.exists() {
            return Ok(None);
        }

        Self::load_from_path(workspace_config_path)
    }

    pub fn path_from_name(name: &Slug) -> eyre::Result<PathBuf> {
        let project_dirs = get_project_dirs()?;

        let filename = format!("{}.toml", name);
        let path = project_dirs
            .config_local_dir()
            .join("workspaces")
            .join(&filename);

        Ok(path)
    }

    pub fn load_from_path(path: PathBuf) -> eyre::Result<Option<Self>> {
        let config_str = std::fs::read_to_string(&path)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to read workspace config from {}", path.display()))?;

        let config: WorkspaceConfig = toml::from_str(&config_str)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to parse workspace config")?;

        Ok(Some(Self {
            config_path: path,
            config,
        }))
    }

    pub fn current() -> eyre::Result<Option<Self>> {
        let project = Project::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load current project")?;

        let Some(project) = project else {
            return Ok(None);
        };

        let workspace_name = project.manifest().workspace().name.clone();
        let workspace = Self::load_from_name(&workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to load workspace {}", workspace_name))?
            .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;

        Ok(Some(workspace))
    }

    pub fn config(&self) -> &WorkspaceConfig {
        &self.config
    }

    pub fn config_path(&self) -> &Path {
        self.config_path.as_path()
    }

    pub fn save(&self) -> eyre::Result<()> {
        // Ensure the parent directory exists
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!(
                        "Failed to create parent directory for {}",
                        self.config_path.display()
                    )
                })?;
        }

        let config_str = toml::to_string_pretty(&self.config)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to format workspace config as string")?;

        std::fs::write(&self.config_path, config_str)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to write workspace config to {}",
                    self.config_path.display()
                )
            })?;

        Ok(())
    }
}
