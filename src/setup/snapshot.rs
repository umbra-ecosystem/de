use eyre::{WrapErr, eyre};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    project::Project,
    setup::{export::ExportCommandResult, utils::EnvMapper},
    types::Slug,
    workspace::Workspace,
};

use super::{
    export::ExportCommand,
    project::{ApplyCommand, StandardStep, StepKind, StepService},
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

pub fn create_snapshot(workspace: Workspace, profile: Slug) -> eyre::Result<(TempDir, Snapshot)> {
    info!("Creating snapshot for workspace with profile '{}'", profile);

    let snapshot_dir = tempfile::tempdir()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to create temporary dir")?;

    let files_dir = snapshot_dir.path().join("files");

    let mut project_snapshots = BTreeMap::new();
    for (name, ws_project) in workspace.config().projects.iter() {
        info!("Loading project '{}'", name);
        let project = Project::from_dir(&ws_project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to load project from {}", ws_project.dir.display())
            })?;

        info!("Creating snapshot for project '{}'", name);
        let project_snapshot = create_project_snapshot(name, &project, &profile, &files_dir)?;
        if let Some(project_snapshot) = project_snapshot {
            project_snapshots.insert(name.clone(), project_snapshot);
        }
    }

    info!("Snapshot creation complete.");
    Ok((
        snapshot_dir,
        Snapshot {
            projects: project_snapshots,
            created_at: Utc::now(),
        },
    ))
}

pub fn create_project_snapshot(
    project_name: &Slug,
    project: &Project,
    profile: &Slug,
    files_dir: &Path,
) -> eyre::Result<Option<ProjectSnapshot>> {
    let Some(project_setup) = project.manifest().setup.as_ref() else {
        info!(
            "No setup found for project '{}', skipping snapshot.",
            project_name
        );
        return Ok(None);
    };

    let project_files_dir = files_dir.join(project_name.as_str());

    let mut project_snapshot = ProjectSnapshot {
        git: project_setup.git(profile),
        steps: Default::default(),
        files: vec![],
    };

    for (name, setup_step) in project_setup.steps(profile) {
        info!(
            "Processing setup step '{}' for project '{}'",
            name, project_name
        );
        let step = ProjectSnapshotStep {
            name: name.clone(),
            service: setup_step.service.as_ref().map(|v| v.clone_value()),
            optional: setup_step.optional,
            skip_if: setup_step.skip_if.clone(),
            kind: match &setup_step.kind {
                StepKind::Standard(standard_step) => match standard_step {
                    StandardStep::CopyFiles {
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
                        info!(
                            "Running export command '{}' for step '{}' in project '{}'",
                            export_command.as_value().command,
                            name,
                            project_name
                        );
                        let result = export_command
                            .as_value()
                            .run(
                                &project.dir(),
                                env_mapper.as_ref(),
                                &project_files_dir,
                                files_dir,
                            )
                            .map_err(|e| eyre!(e))
                            .wrap_err_with(|| {
                                format!(
                                    "Failed to run export command: {}",
                                    export_command.as_value().command
                                )
                            })?;

                        match result {
                            ExportCommandResult::File { file_path } => {
                                info!(
                                    "Export command produced file '{}' for step '{}' in project '{}'",
                                    file_path.display(),
                                    name,
                                    project_name
                                );
                                project_snapshot.files.push(file_path);
                            }
                            ExportCommandResult::NoOutput => {}
                        }
                    }

                    ProjectSnapshotStepKind::Complex {
                        apply: apply
                            .as_slice()
                            .into_iter()
                            .map(|cmd| cmd.clone_value())
                            .collect(),
                        export: export
                            .as_slice()
                            .into_iter()
                            .map(|cmd| cmd.clone_value())
                            .collect(),
                        env: env.clone(),
                    }
                }
                StepKind::Basic { command, env } => ProjectSnapshotStepKind::Basic {
                    command: command
                        .as_slice()
                        .into_iter()
                        .map(|cmd| cmd.clone_value())
                        .collect(),
                    env: env.clone(),
                },
            },
        };

        project_snapshot.steps.insert(name.clone(), step);
    }

    Ok(Some(project_snapshot))
}
