use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

use crate::types::Slug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub name: Slug,
    /// A workspace that has not had a project added yet simply has none.
    #[serde(default)]
    pub projects: BTreeMap<Slug, WorkspaceProject>,
    #[serde(default)]
    pub tasks: BTreeMap<Slug, String>,
    #[serde(default)]
    pub default_branch: Option<String>,
    /// When the workspace was last opened, selected or closed (unix seconds). `None` for one
    /// that has never been opened. Orders the workspace lists, most recent first.
    #[serde(default)]
    pub last_used: Option<i64>,
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
