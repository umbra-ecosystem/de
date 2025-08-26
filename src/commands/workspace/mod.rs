mod config;
mod info;
mod run;

use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub use config::config;
pub use info::info;
pub use run::run;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

use crate::{
    setup::snapshot::{Snapshot, create_snapshot},
    types::Slug,
    utils::{get_workspace_for_cli, ui::UserInterface},
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

    let manifest_path = snapshot_dir.path().join("manifest.json");
    let manifest_content = serde_json::to_string_pretty(snapshot)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to serialize snapshot manifest for: {workspace_name}"))?;
    let manifest_size = manifest_content.as_bytes().len();

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

fn zip_dir(zip_file: File, dir: &Path) -> eyre::Result<()> {
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);

    let prefix = Path::new(dir);
    let mut buffer = Vec::new();

    for entry in walkdir::WalkDir::new(dir).max_depth(10) {
        let entry = entry
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to read directory entry in: {}", dir.display()))?;

        let path = entry.path();
        let name = path.strip_prefix(prefix).unwrap();
        let path_as_string = name
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| eyre!("{name:?} Is a Non UTF-8 Path"))?;

        if path.is_file() {
            tracing::debug!("Adding file to zip: {path_as_string}");
            zip.start_file(path_as_string, options)?;
            let mut f = File::open(path)?;
            f.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
            buffer.clear();
        } else if !name.as_os_str().is_empty() {
            tracing::debug!("Adding dir to zip: {path_as_string}");
            zip.add_directory(path_as_string, options)?;
        }
    }

    zip.finish()?;

    Ok(())
}
