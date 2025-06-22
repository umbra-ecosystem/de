use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use eyre::{Context, eyre};
use serde::{Deserialize, Serialize};

use crate::types::Slug;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectManifest {
    #[serde(default)]
    pub workspace: WorkspaceManifest,
    #[serde(default)]
    pub project: Option<ProjectMetadata>,
}

impl ProjectManifest {
    pub fn workspace(&self) -> &WorkspaceManifest {
        &self.workspace
    }

    pub fn project(&self) -> Option<&ProjectMetadata> {
        self.project.as_ref()
    }
}

impl ProjectManifest {
    pub fn from_file(path: &Path) -> eyre::Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to read project manifest from {}", path.display()))?;

        let manifest: Self = toml::from_str(&content)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to parse project manifest")?;

        Ok(Some(manifest))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    #[serde(default = "default_workspace_name")]
    pub name: Slug,
}

impl Default for WorkspaceManifest {
    fn default() -> Self {
        Self {
            name: default_workspace_name(),
        }
    }
}

fn default_workspace_name() -> Slug {
    Slug::from_str("default").expect("default workspace name should be valid")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectMetadata {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub docker_compose: Option<PathBuf>,
}
