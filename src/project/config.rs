use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    str::FromStr,
};

use eyre::{Context, eyre};
use serde::{Deserialize, Serialize};

use crate::{project::task::Task, types::Slug};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectManifest {
    #[serde(default)]
    pub project: ProjectMetadata,
    #[serde(default)]
    pub git: Option<ProjectGitSettings>,
    #[serde(default)]
    pub tasks: Option<BTreeMap<Slug, Task>>,
}

impl ProjectManifest {
    pub fn project(&self) -> &ProjectMetadata {
        &self.project
    }

    pub fn load(path: &Path) -> eyre::Result<ProjectManifest> {
        let manifest_str = std::fs::read_to_string(path)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to read manifest file at {}", path.display()))?;

        toml::from_str(&manifest_str)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to parse project manifest")
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let manifest_str = toml::to_string_pretty(&self)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to format manifest as string")?;

        std::fs::write(path, manifest_str)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to write manifest to {}", path.display()))?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    #[serde(default = "default_project_name")]
    pub name: Slug,
    #[serde(default = "default_project_workspace")]
    pub workspace: Slug,
    #[serde(default)]
    pub docker_compose: Option<PathBuf>,
    #[serde(default)]
    pub depends_on: Option<Vec<Slug>>,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            name: default_project_name(),
            workspace: default_project_workspace(),
            docker_compose: Default::default(),
            depends_on: Default::default(),
        }
    }
}

fn default_project_name() -> Slug {
    Slug::from_str("default").expect("default project name should be valid")
}

fn default_project_workspace() -> Slug {
    Slug::from_str("default").expect("default workspace name should be valid")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGitSettings {
    #[serde(default = "default_git_enabled")]
    pub enabled: bool,
    #[serde(default = "default_git_remote")]
    pub default_remote: String,
}

impl Default for ProjectGitSettings {
    fn default() -> Self {
        Self {
            enabled: default_git_enabled(),
            default_remote: default_git_remote(),
        }
    }
}

fn default_git_enabled() -> bool {
    true
}

fn default_git_remote() -> String {
    "origin".to_string()
}
