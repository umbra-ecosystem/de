use std::{collections::BTreeMap, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{project::task::Task, types::Slug};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectManifest {
    #[serde(default)]
    pub workspace: WorkspaceManifest,
    #[serde(default)]
    pub project: Option<ProjectMetadata>,
    #[serde(default)]
    pub tasks: Option<BTreeMap<Slug, Task>>,
}

impl ProjectManifest {
    pub fn workspace(&self) -> &WorkspaceManifest {
        &self.workspace
    }

    pub fn project(&self) -> Option<&ProjectMetadata> {
        self.project.as_ref()
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
