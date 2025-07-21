pub mod config;
mod dependency;
mod utils;

use eyre::{Context, eyre};
use std::path::PathBuf;

use crate::{
    config::Config, project::Project, types::Slug, utils::get_project_dirs,
    workspace::config::WorkspaceConfig,
};

pub use config::WorkspaceProject;
pub use dependency::{DependencyGraph, DependencyGraphError};
pub use utils::{add_project_to_workspace, spin_down_workspace, spin_up_workspace};

#[derive(Debug)]
pub struct Workspace {
    config: WorkspaceConfig,
    pub config_path: PathBuf,
}

impl Workspace {
    pub fn new(name: Slug) -> eyre::Result<Self> {
        let config_path = Self::path_from_name(&name)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to create path for workspace {name}"))?;

        let config = WorkspaceConfig {
            name,
            projects: Default::default(),
            tasks: Default::default(),
            default_branch: Default::default(),
        };

        Ok(Self {
            config,
            config_path,
        })
    }

    pub fn add_project(&mut self, id: Slug, project: WorkspaceProject) {
        self.config.projects.insert(id, project);
    }

    pub fn remove_project(&mut self, id: &Slug) {
        self.config.projects.remove(id);
    }

    pub fn load_dependency_graph(&self) -> eyre::Result<(DependencyGraph, Vec<Project>)> {
        let mut graph = DependencyGraph::new();
        let mut projects = Vec::new();

        for (id, ws_project) in &self.config.projects {
            let project = Project::from_dir(&ws_project.dir)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!("Failed to load project from {}", ws_project.dir.display())
                })?;

            if let Some(depends_on) = &project.manifest().project().depends_on {
                graph.add_project(id.clone(), depends_on.clone());
            } else {
                graph.add_project(id.clone(), Vec::new());
            }

            projects.push(project);
        }

        Ok((graph, projects))
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

        let filename = format!("{name}.toml");
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

    pub fn config(&self) -> &WorkspaceConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut WorkspaceConfig {
        &mut self.config
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

impl Workspace {
    pub fn current() -> eyre::Result<Option<Self>> {
        let project = Project::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load current project")?;

        let Some(project) = project else {
            return Ok(None);
        };

        let workspace_name = project.manifest().project().workspace.clone();
        let workspace = Self::load_from_name(&workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to load workspace {workspace_name}"))?
            .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;

        Ok(Some(workspace))
    }

    pub fn working() -> eyre::Result<Option<Self>> {
        let app_config = Config::load()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load application config")?;

        let Some(workspace_name) = app_config.get_active_workspace() else {
            return Ok(None);
        };

        let workspace = Self::load_from_name(workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to load workspace {workspace_name}"))?;

        Ok(workspace)
    }

    pub fn active() -> eyre::Result<Option<Self>> {
        // Try to get the current workspace from the environment
        let current_workspace = Self::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current workspace")?;

        if let Some(workspace) = current_workspace {
            return Ok(Some(workspace));
        }

        // If no current workspace, try to load the workspace from the config
        let working_workspace = Self::working()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get working workspace")?;

        if let Some(workspace) = working_workspace {
            return Ok(Some(workspace));
        }

        Ok(None)
    }
}
