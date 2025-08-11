use std::{collections::BTreeMap, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::project::config::{ApplyCommand, ExportCommand, StepService};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Snapshot {
    pub projects: Vec<SnapshotProject>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnapshotProject {
    pub steps: Vec<SnapshotProjectStep>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnapshotProjectStep {
    pub name: String,
    pub service: Option<StepService>,
    pub optional: Option<bool>,
    pub skip_if: Option<String>,
    pub kind: SnapshotProjectStepKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SnapshotProjectStepKind {
    CopyFiles {
        source: String,
        destination: String,
        overwrite: bool,
        files: Vec<CopyFile>,
    },
    Complex {
        apply: Vec<ApplyCommand>,
        export: Vec<ExportCommand>,
        env: Option<BTreeMap<String, String>>,
        content: Option<BTreeMap<String, ComplexContent>>,
    },
    Basic {
        command: Vec<ApplyCommand>,
        env: Option<BTreeMap<String, String>>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CopyFile {
    stored_path: PathBuf,
    destination_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ComplexContent {
    File(PathBuf),
    Inline(String),
}
