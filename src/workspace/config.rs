use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::types::Slug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub name: Slug,
    pub projects: Vec<WorkspaceProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceProject {
    pub manifest: PathBuf,
}

impl WorkspaceProject {
    pub fn from_manifest_path(manifest_path: PathBuf) -> eyre::Result<Self> {
        Ok(Self {
            manifest: manifest_path,
        })
    }
}
