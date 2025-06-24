use std::{collections::BTreeMap, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{project::task::Task, types::Slug};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectManifest {
    #[serde(default)]
    pub workspace: WorkspaceManifest,
    #[serde(default)]
    pub project: ProjectMetadata,
    #[serde(default)]
    pub tasks: Option<BTreeMap<Slug, Task>>,
}

impl ProjectManifest {
    pub fn workspace(&self) -> &WorkspaceManifest {
        &self.workspace
    }

    pub fn project(&self) -> &ProjectMetadata {
        &self.project
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    #[serde(default = "default_project_name")]
    pub name: Slug,
    #[serde(default)]
    pub docker_compose: Option<PathBuf>,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            name: Slug::from_str("default").expect("default project name should be valid"),
            docker_compose: None,
        }
    }
}

fn default_project_name() -> Slug {
    Slug::from_str("default").expect("default project name should be valid")
}
