mod config;
mod info;
mod run;

use std::{fs::File, path::PathBuf};

pub use config::config;
pub use info::info;
pub use run::run;
use tempfile::TempDir;

use crate::{
    setup::snapshot::{SNAPSHOT_MANIFEST_FILE, Snapshot, create_snapshot},
    types::Slug,
    utils::{get_workspace_for_cli, ui::UserInterface, zip::zip_dir},
};
use eyre::{WrapErr, eyre};

pub fn snapshot(workspace_name: Option<Slug>, profile: Slug) -> eyre::Result<()> {
    let workspace = get_workspace_for_cli(Some(workspace_name))?;
    let workspace_name = workspace.config().name.clone();

    let ui = UserInterface::new();

    let (snapshot_dir, snapshot) = create_snapshot(&ui, workspace, profile)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to create snapshot for workspace: {workspace_name}"))?;

    ui.new_line()?;
    zip_snapshot(&ui, &workspace_name, &snapshot_dir, &snapshot)?;

    Ok(())
}

fn zip_snapshot(
    ui: &UserInterface,
    workspace_name: &Slug,
    snapshot_dir: &TempDir,
    snapshot: &Snapshot,
) -> eyre::Result<()> {
    ui.heading("Bundle")?;

    let manifest_path = snapshot_dir.path().join(SNAPSHOT_MANIFEST_FILE);
    let manifest_content = serde_json::to_string_pretty(snapshot)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to serialize snapshot manifest for: {workspace_name}"))?;
    let manifest_size = manifest_content.len();

    std::fs::write(&manifest_path, manifest_content)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to write manifest for snapshot: {workspace_name}"))?;

    let manifest_name = manifest_path
        .strip_prefix(snapshot_dir.path())
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| {
            format!("Failed to get relative path for manifest in snapshot: {workspace_name}")
        })?;

    ui.success_item(
        &format!(
            "Manifest: {} {}",
            manifest_name.display(),
            ui.theme.dim(&format!("({manifest_size}b)"))
        ),
        None,
    )?;

    let zip_path = PathBuf::from(format!("{workspace_name}.zip"));
    let zip_file = File::create(&zip_path)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to create zip file: {}", zip_path.display()))?;

    zip_dir(zip_file, snapshot_dir.path())
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to zip snapshot directory for: {workspace_name}"))?;

    let zip_size = std::fs::metadata(&zip_path)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to get metadata for zip file: {}",
                zip_path.display()
            )
        })?
        .len();

    ui.success_item(
        &format!(
            "Output: {} {}",
            zip_path.display(),
            ui.theme.dim(&format!("({zip_size}b)"))
        ),
        None,
    )?;

    Ok(())
}
