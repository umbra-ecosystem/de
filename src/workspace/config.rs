use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

use crate::types::Slug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub name: Slug,
    pub projects: BTreeMap<Slug, WorkspaceProject>,
    #[serde(default)]
    pub tasks: BTreeMap<Slug, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTask {
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceProject {
    pub dir: PathBuf,
}

impl WorkspaceProject {
    pub fn new(dir: PathBuf) -> eyre::Result<Self> {
        Ok(Self { dir })
    }
}
