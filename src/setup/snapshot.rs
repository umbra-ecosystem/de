use eyre::{WrapErr, eyre};
use std::{collections::BTreeMap, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{project::Project, setup::utils::EnvMapper, types::Slug, workspace::Workspace};

use super::{
    export::ExportCommand,
    project::{ApplyCommand, StepKind, StepService},
    types::GitConfig,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Snapshot {
    pub projects: BTreeMap<Slug, ProjectSnapshot>,
    pub created_at: DateTime<Utc>,
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
        export: Vec<ExportCommand>,
        env: Option<BTreeMap<String, String>>,
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

pub fn create_snapshot(workspace: Workspace) -> eyre::Result<Snapshot> {
    let mut project_snapshots = BTreeMap::new();
    for (name, ws_project) in workspace.config().projects.iter() {
        let project = Project::from_dir(&ws_project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to load project from {}", ws_project.dir.display())
            })?;

        let project_snapshot = create_project_snapshot(&project)?;
        if let Some(project_snapshot) = project_snapshot {
            project_snapshots.insert(name.clone(), project_snapshot);
        }
    }

    Ok(Snapshot {
        projects: project_snapshots,
        created_at: Utc::now(),
    })
}

pub fn create_project_snapshot(project: &Project) -> eyre::Result<Option<ProjectSnapshot>> {
    let Some(project_setup) = project.manifest().setup.as_ref() else {
        return Ok(None);
    };

    let mut project_snapshot = ProjectSnapshot {
        git: project_setup.git.clone_value(),
        steps: Default::default(),
        files: vec![],
    };

    for (name, setup_step) in project_setup.steps.iter() {
        let step = ProjectSnapshotStep {
            name: name.clone(),
            service: setup_step.service.as_ref().map(|v| v.clone_value()),
            optional: setup_step.optional,
            skip_if: setup_step.skip_if.clone(),
            kind: match &setup_step.kind {
                StepKind::Standard(standard_step) => match standard_step {
                    super::project::StandardStep::CopyFiles {
                        source,
                        destination,
                        overwrite,
                    } => ProjectSnapshotStepKind::CopyFiles {
                        source: source.clone(),
                        destination: destination.clone(),
                        overwrite: *overwrite,
                    },
                },
                StepKind::Complex { apply, export, env } => {
                    let env_mapper = env.as_ref().map(EnvMapper::new);
                    for export_command in export.as_slice() {
                        let result = export_command.as_value();
                    }
                    todo!("Implement complex step handling")
                }
                StepKind::Basic { command, env } => todo!(),
            },
        };

        project_snapshot.steps.insert(name.clone(), step);
    }

    Ok(Some(project_snapshot))
}
