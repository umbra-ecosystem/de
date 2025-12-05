use std::{collections::BTreeMap, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    setup::{
        project::StepService,
        snapshot::checksum::SnapshotChecksum,
        types::{ApplyCommand, GitConfig},
    },
    types::Slug,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Snapshot {
    pub workspace: WorkspaceSnapshot,
    pub projects: BTreeMap<Slug, ProjectSnapshot>,
    pub checksum: Option<SnapshotChecksum>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkspaceSnapshot {
    pub name: Slug,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectSnapshot {
    pub git: GitConfig,
    pub steps: BTreeMap<Slug, ProjectSnapshotStep>,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectSnapshotStep {
    pub name: Slug,
    pub service: Option<StepService>,
    pub optional: bool,
    pub skip_if: Option<String>,
    pub kind: ProjectSnapshotStepKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSnapshotStepKind {
    CopyFiles {
        source: String,
        destination: String,
        overwrite: bool,
    },
    Complex {
        apply: Vec<ApplyCommand>,
    },
    Basic {
        command: Vec<ApplyCommand>,
    },
}

impl ProjectSnapshotStepKind {
    pub fn as_str(&self) -> &str {
        match self {
            ProjectSnapshotStepKind::CopyFiles { .. } => "copy_files",
            ProjectSnapshotStepKind::Complex { .. } => "complex",
            ProjectSnapshotStepKind::Basic { .. } => "basic",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CopyFile {
    stored_path: PathBuf,
    destination_path: PathBuf,
}
