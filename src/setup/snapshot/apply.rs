use std::{fs::File, path::Path};

use eyre::Context;
use walkdir::WalkDir;

use crate::{
    setup::{
        snapshot::{
            SNAPSHOT_MANIFEST_FILE, Snapshot,
            types::{ProjectSnapshot, ProjectSnapshotStep},
        },
        types::{ApplyCommand, CommandPipe},
    },
    types::Slug,
    utils::{git::run_git_command, ui::UserInterface, zip::extract_zip},
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

    let canonical_snapshot_dir = snapshot_dir
        .path()
        .canonicalize()
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to canonicalize snapshot directory: {}",
                snapshot_dir.path().display()
            )
        })?;

    ui.heading("Projects")?;
    for (project_name, project_snapshot) in snapshot.projects.iter() {
        ui.subheading(&format!("{}", project_name))?;
        ui.indented(|ui| {
            apply_project_snapshot(
                &ui,
                &canonical_snapshot_dir,
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

    ui.writeln(&format!("{} git", ui.theme.dim("0")))?;
    ui.indented(|ui| {
        project_step_git(
            ui,
            project_name,
            project_snapshot,
            &project_dir,
            &target_dir,
        )
    })?;

    for (step_index, (_, step_snapshot)) in project_snapshot.steps.iter().enumerate() {
        ui.writeln(&format!(
            "{} {} {}",
            step_index + 1,
            step_snapshot.name,
            ui.theme.dim(&format!("({})", step_snapshot.kind.as_str())),
        ))?;

        ui.indented(|ui| {
            apply_project_step(ui, snapshot_dir, &project_dir, step_snapshot)?;
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
    snapshot_dir: &Path,
    project_dir: &Path,
    step_snapshot: &ProjectSnapshotStep,
) -> eyre::Result<()> {
    match &step_snapshot.kind {
        ProjectSnapshotStepKind::CopyFiles {
            source,
            destination,
            overwrite,
        } => {
            apply_project_step_copy_files(ui, project_dir, source, destination, *overwrite)?;
        }
        ProjectSnapshotStepKind::Basic { command } => {
            for cmd in command {
                run_apply_command(ui, snapshot_dir, project_dir, cmd)?;
            }
        }
        ProjectSnapshotStepKind::Complex { apply } => {
            for cmd in apply {
                run_apply_command(ui, snapshot_dir, project_dir, cmd)?;
            }
        }
    }

    Ok(())
}

fn apply_project_step_copy_files(
    ui: &UserInterface,
    project_dir: &Path,
    source: &str,
    destination: &str,
    overwrite: bool,
) -> eyre::Result<()> {
    ui.info_item(&format!(
        "Processing {} -> {}",
        ui.theme.accent(source),
        ui.theme.accent(destination)
    ))?;

    let source_re = regex::Regex::new(source)
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| format!("Invalid source regex: {}", source))?;

    let mut matched_files = 0;

    ui.indented(|ui| {
        for entry in WalkDir::new(project_dir).max_depth(255) {
            let entry = entry.map_err(|e| eyre::eyre!(e)).wrap_err_with(|| {
                format!(
                    "Failed to read files in project directory: {}",
                    project_dir.display()
                )
            })?;

            let name = if let Some(name) = entry.file_name().to_str() {
                name
            } else {
                tracing::warn!("Skipping non-UTF8 file name: {:?}", entry.file_name());
                continue;
            };

            if !source_re.is_match(name) {
                continue;
            };

            let dest_name = source_re.replace(name, destination).to_string();
            let parent = if let Some(parent) = entry.path().parent() {
                parent
            } else {
                tracing::warn!("Skipping file with no parent: {:?}", entry.path());
                continue;
            };

            let dest_path = parent.join(dest_name);
            if dest_path.exists() && !overwrite {
                ui.warning_item(
                    &format!(
                        "Skipping existing file: {}",
                        ui.theme.dim(&dest_path.display().to_string())
                    ),
                    None,
                )?;

                continue;
            }

            std::fs::copy(entry.path(), &dest_path)
                .map_err(|e| eyre::eyre!(e))
                .wrap_err_with(|| {
                    format!(
                        "Failed to copy file from {} to {}",
                        entry.path().display(),
                        dest_path.display()
                    )
                })?;

            ui.success_item(
                &format!(
                    "{} -> {}",
                    &entry.path().display().to_string(),
                    ui.theme.accent(&dest_path.display().to_string()),
                ),
                None,
            )?;

            matched_files += 1;
        }

        Ok(())
    })?;

    if matched_files == 0 {
        ui.warning_item(
            &format!("No files matched source pattern: {}", ui.theme.dim(source)),
            None,
        )?;
    }

    Ok(())
}

fn run_apply_command(
    ui: &UserInterface,
    snapshot_dir: &Path,
    project_dir: &Path,
    apply_command: &ApplyCommand,
) -> eyre::Result<()> {
    use std::process::{Command, Stdio};

    ui.info_item(&format!(
        "Running command: {}",
        ui.theme.accent(&apply_command.to_string())
    ))?;

    let mut parts = apply_command.command.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| eyre::eyre!("Command is empty"))?;

    let mut command = Command::new(program);
    command.current_dir(project_dir);
    command.args(parts);

    if let Some(stdin_pipe) = &apply_command.stdin {
        match stdin_pipe {
            CommandPipe::File { file } => {
                tracing::info!("Using file '{}' as stdin", file);

                let file_path = snapshot_dir
                    .join(file)
                    .canonicalize()
                    .map_err(|e| eyre::eyre!(e))
                    .wrap_err_with(|| {
                        format!("Failed to canonicalize stdin file path: {}", file)
                    })?;

                // SECURITY: Ensure the file is within the snapshot directory
                if !file_path.starts_with(snapshot_dir) {
                    return Err(eyre::eyre!(
                        "Stdin file path '{}' is outside of snapshot directory: {}",
                        snapshot_dir.display(),
                        file_path.display()
                    ));
                }

                let input = std::fs::File::open(&file_path)
                    .map_err(|e| eyre::eyre!(e))
                    .wrap_err_with(|| {
                        format!("Failed to open stdin file: {}", file_path.display())
                    })?;

                command.stdin(Stdio::from(input));
            }
        }
    }

    let status = command
        .status()
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| format!("Failed to run command: {}", apply_command.command))?;

    if !status.success() {
        ui.error_item(
            &format!(
                "Command failed: {} (status: {})",
                apply_command.command, status
            ),
            None,
        )?;

        return Err(eyre::eyre!("Command failed with status: {}", status));
    }

    ui.success_item(
        &format!(
            "Command succeeded: {}",
            ui.theme.accent(&apply_command.command)
        ),
        None,
    )?;

    Ok(())
}
