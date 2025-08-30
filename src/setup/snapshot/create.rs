use eyre::{WrapErr, eyre};
use std::{collections::BTreeMap, path::Path};
use tempfile::TempDir;

use chrono::Utc;

use crate::{
    project::Project,
    setup::{
        export::ExportCommandResult,
        project::{StandardStep, StepKind},
        snapshot::types::{
            ProjectSnapshot, ProjectSnapshotStep, ProjectSnapshotStepKind, Snapshot,
            WorkspaceSnapshot,
        },
        utils::EnvMapper,
    },
    types::Slug,
    utils::ui::UserInterface,
    workspace::Workspace,
};

pub fn create_snapshot(
    ui: &UserInterface,
    workspace: Workspace,
    profile: Slug,
) -> eyre::Result<(TempDir, Snapshot)> {
    tracing::info!("Creating snapshot for workspace with profile '{}'", profile);

    ui.heading("Snapshot Creation")?;
    ui.info_item(&format!("workspace: {}", workspace.config().name))?;
    ui.new_line()?;

    let workspace_snapshot = WorkspaceSnapshot {
        name: workspace.config().name.clone(),
    };

    let snapshot_dir = tempfile::tempdir()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to create temporary dir")?;

    let files_dir = snapshot_dir.path().join("files");

    ui.heading("Projects")?;

    let mut project_snapshots = BTreeMap::new();
    for (name, ws_project) in workspace.config().projects.iter() {
        tracing::info!("Loading project '{}'", name);
        let project = Project::from_dir(&ws_project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to load project from {}", ws_project.dir.display())
            })?;

        tracing::info!("Creating snapshot for project '{}'", name);
        let project_snapshot = create_project_snapshot(
            ui,
            name,
            &project,
            &profile,
            &files_dir,
            snapshot_dir.path(),
        )?;
        if let Some(project_snapshot) = project_snapshot {
            project_snapshots.insert(name.clone(), project_snapshot);
        }
    }

    tracing::info!("Snapshot creation complete.");
    Ok((
        snapshot_dir,
        Snapshot {
            workspace: workspace_snapshot,
            projects: project_snapshots,
            created_at: Utc::now(),
        },
    ))
}

pub fn create_project_snapshot(
    ui: &UserInterface,
    project_name: &Slug,
    project: &Project,
    profile: &Slug,
    files_dir: &Path,
    prefix_dir: &Path,
) -> eyre::Result<Option<ProjectSnapshot>> {
    let step_count = project
        .manifest()
        .setup
        .as_ref()
        .map_or(0, |setup| setup.steps.len());

    ui.subheading(&format!(
        "{} {}",
        project_name.as_str(),
        ui.theme.dim(&format!(
            "({} {})",
            &step_count.to_string(),
            if step_count == 1 { "step" } else { "steps" }
        )),
    ))?;

    let Some(project_setup) = project.manifest().setup.as_ref() else {
        tracing::info!(
            "No setup found for project '{}', skipping snapshot.",
            project_name
        );

        ui.indented(|ui| {
            ui.warning_item("No setup found, skipping...", None)?;
            Ok(())
        })?;

        return Ok(None);
    };

    let project_files_dir = files_dir.join(project_name.as_str());

    let mut project_snapshot = ProjectSnapshot {
        git: project_setup.git(profile),
        steps: Default::default(),
        files: vec![],
    };

    ui.indented(|ui| {
        for (i, (name, setup_step)) in project_setup.steps(profile).iter().enumerate() {
            tracing::info!(
                "Processing setup step '{}' for project '{}'",
                name,
                project_name
            );

            ui.writeln(&format!("{} {} {}", ui.theme.dim((i + 1).to_string().as_str()), setup_step.name, ui.theme.dim(&format!("({})", setup_step.kind.as_str()))))?;

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
                        } => {
                            ui.indented(|ui| {
                                ui.info_item("No preprocessing required")?;
                                Ok(())
                            })?;

                            ProjectSnapshotStepKind::CopyFiles {
                                source: source.clone(),
                                destination: destination.clone(),
                                overwrite: *overwrite,
                            }
                        },
                    },
                    StepKind::Complex { apply, export, env } => {
                        let env_mapper = env.as_ref().map(EnvMapper::new);

                        ui.indented(|ui| {
                            for export_command in export.as_slice() {
                                tracing::info!(
                                    "Running export command '{}' for step '{}' in project '{}'",
                                    export_command.as_value().command,
                                    name,
                                    project_name
                                );

                                let resolved_command = export_command
                                    .as_value()
                                    .resolve_env(env_mapper.as_ref());

                                ui.info_item(&format!(
                                    "Running export command: {}",
                                    ui.theme.accent(&resolved_command.command)
                                ))?;

                                let result = resolved_command
                                    .run(project.dir(), &project_files_dir, prefix_dir)
                                    .map_err(|e| eyre!(e))
                                    .wrap_err_with(|| {
                                        format!(
                                            "Failed to run export command: {}",
                                            export_command.as_value().command
                                        )
                                    })?;

                                ui.indented(|ui| {
                                    match result {
                                        ExportCommandResult::File { file_path } => {
                                            tracing::info!(
                                                "Export command produced file '{}' for step '{}' in project '{}'",
                                                file_path.display(),
                                                name,
                                                project_name
                                            );

                                            ui.success_item(&format!(
                                                "Exported file: {}",
                                                ui.theme.accent(&file_path.display().to_string())
                                            ), None)?;

                                            project_snapshot.files.push(file_path);
                                        }
                                        ExportCommandResult::NoOutput => {}
                                    }

                                    Ok(())
                                })?;
                            }


                            let apply_vec = apply
                                .as_slice()
                                .iter()
                                .map(|cmd| cmd.as_value().resolve_env(env_mapper.as_ref()))
                                .collect::<Vec<_>>();

                            for cmd in apply_vec.iter() {
                                ui.info_item(&format!(
                                    "Apply Command: {}",
                                    ui.theme.accent(&cmd.command)
                                ))?;
                            }

                            Ok(ProjectSnapshotStepKind::Complex {
                                apply: apply_vec,
                            })
                        })?
                    }
                    StepKind::Basic { command, env } => {
                        let env_mapper = env.as_ref().map(EnvMapper::new);

                        let command_vec = command
                            .as_slice()
                            .iter()
                            .map(|cmd| cmd.as_value().resolve_env(env_mapper.as_ref()))
                            .collect::<Vec<_>>();

                        ui.indented(|ui| {
                            for cmd in command_vec.iter() {
                                ui.info_item(&format!(
                                    "Command: {}",
                                    ui.theme.accent(&cmd.command)
                                ))?;
                            }
                            Ok(())
                        })?;

                        ProjectSnapshotStepKind::Basic {
                            command: command_vec,
                        }
                    }
                },
            };

            project_snapshot.steps.insert(name.clone(), step);
        }

        Ok(())
    })?;

    Ok(Some(project_snapshot))
}
