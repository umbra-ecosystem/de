use eyre::eyre;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::{manifest::Project, types::Slug};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub name: Slug,
    pub projects: Vec<WorkspaceProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceProject {
    pub name: String,
    pub manifest_path: PathBuf,
}

impl WorkspaceProject {
    pub fn from_manifest_path(manifest_path: PathBuf) -> eyre::Result<Self> {
        let project = Project::from_manifest_path(manifest_path.clone())
            .expect("Failed to create project from manifest path");

        let name = if let Some(name) = project.manifest().project().and_then(|p| p.name.as_deref())
        {
            name.to_string()
        } else {
            manifest_path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|f| f.to_str())
                .ok_or_else(|| eyre!("Failed to extract project name from manifest path"))?
                .to_string()
        };

        Ok(Self {
            name,
            manifest_path,
        })
    }
}
