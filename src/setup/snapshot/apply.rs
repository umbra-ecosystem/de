use std::{fs::File, path::Path};

use eyre::Context;

use crate::{
    setup::snapshot::{
        SNAPSHOT_MANIFEST_FILE, Snapshot,
        types::{ProjectSnapshot, ProjectSnapshotStep},
    },
    types::Slug,
    utils::{git::run_git_command, ui::UserInterface, zip::extract_zip},
    workspace::Workspace,
};

use super::types::ProjectSnapshotStepKind;

pub fn apply_snapshot(
    ui: &UserInterface,
    snapshot_path: &Path,
    target_dir: &Path,
) -> eyre::Result<()> {
    ui.heading("Apply Snapshot")?;
    let loading_bar = ui.loading_bar("Preparing...")?;
    let snapshot_dir = extract_snapshot_to_tempdir(snapshot_path)?;
    let snapshot = read_snapshot_manifest(snapshot_dir.path())?;
    loading_bar.finish_and_clear();

    ui.info_item(&format!("workspace: {}", snapshot.workspace.name))?;
    ui.info_item(&format!("created at: {}", snapshot.created_at))?;
    ui.new_line()?;

    ui.heading("Projects")?;
    for (project_name, project_snapshot) in snapshot.projects.iter() {
        ui.subheading(&format!("{}", project_name))?;
        ui.indented(|ui| {
            apply_project_snapshot(
                &ui,
                snapshot_dir.path(),
                &snapshot,
                project_name,
                project_snapshot,
                target_dir,
            )?;
            Ok(())
        })?;
    }

    Ok(())
}

fn extract_snapshot_to_tempdir(snapshot_path: &Path) -> eyre::Result<tempfile::TempDir> {
    let temp_dir = tempfile::tempdir()
        .map_err(|e| eyre::eyre!(e))
        .wrap_err("Failed to create temporary dir")?;

    let snapshot_file = File::open(snapshot_path)
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| format!("Failed to open snapshot file: {}", snapshot_path.display()))?;

    extract_zip(snapshot_file, temp_dir.path())
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to extract snapshot file: {}",
                snapshot_path.display()
            )
        })?;

    Ok(temp_dir)
}

fn read_snapshot_manifest(snapshot_dir: &Path) -> eyre::Result<Snapshot> {
    let manifest_path = snapshot_dir.join(SNAPSHOT_MANIFEST_FILE);
    let manifest_content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| format!("Failed to read manifest file: {}", manifest_path.display()))?;

    let snapshot: Snapshot = serde_json::from_str(&manifest_content)
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| format!("Failed to parse manifest file: {}", manifest_path.display()))?;

    Ok(snapshot)
}

fn apply_project_snapshot(
    ui: &UserInterface,
    snapshot_dir: &Path,
    snapshot: &Snapshot,
    project_name: &Slug,
    project_snapshot: &ProjectSnapshot,
    target_dir: &Path,
) -> eyre::Result<()> {
    let project_dir = target_dir.join(project_name.as_str());
    std::fs::create_dir_all(&project_dir)
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to create project directory: {}",
                project_dir.display()
            )
        })?;

    ui.writeln(&format!("0 git"))?;
    ui.indented(|ui| {
        project_step_git(
            ui,
            project_name,
            project_snapshot,
            &project_dir,
            &target_dir,
        )
    })?;

    for (step_index, (step_name, step_snapshot)) in project_snapshot.steps.iter().enumerate() {
        ui.writeln(&format!(
            "{} {} {}",
            step_index + 1,
            step_snapshot.name,
            ui.theme.dim(&format!("({})", step_snapshot.kind.as_str())),
        ))?;

        ui.indented(|ui| {
            apply_project_step(ui, &project_dir, step_name, step_snapshot)?;
            Ok(())
        })?;
    }

    Ok(())
}

fn project_step_git(
    ui: &UserInterface,
    project_name: &Slug,
    project_snapshot: &ProjectSnapshot,
    project_dir: &Path,
    target_dir: &Path,
) -> eyre::Result<()> {
    ui.info_item(&format!(
        "Cloning {}",
        ui.theme.accent(project_snapshot.git.url.as_str())
    ))?;

    run_git_command(
        &[
            "clone",
            project_snapshot.git.url.as_str(),
            project_name.as_str(),
        ],
        target_dir,
    )?;

    // Checkout the specific branch or commit
    if let Some(branch) = &project_snapshot.git.branch {
        ui.info_item(&format!("Branch {}", ui.theme.accent(branch)))?;
        run_git_command(&["checkout", branch.as_str()], project_dir)?;
    }

    Ok(())
}

fn apply_project_step(
    ui: &UserInterface,
    project_dir: &Path,
    step_name: &Slug,
    step_snapshot: &ProjectSnapshotStep,
) -> eyre::Result<()> {
    return Ok(());

    match &step_snapshot.kind {
        ProjectSnapshotStepKind::CopyFiles {
            source,
            destination,
            overwrite,
        } => todo!(),
        ProjectSnapshotStepKind::Complex { apply } => todo!(),
        ProjectSnapshotStepKind::Basic { command } => todo!(),
    }

    Ok(())
}
