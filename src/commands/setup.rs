use std::path::{Path, PathBuf};

use eyre::Context;

use crate::{setup::snapshot::apply_snapshot, utils::ui::UserInterface};

pub fn setup(snapshot: PathBuf, target_dir: Option<PathBuf>) -> eyre::Result<()> {
    let target_dir = if let Some(dir) = target_dir {
        // Create the directory if it doesn't exist
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .map_err(|e| eyre::eyre!(e))
                .wrap_err_with(|| {
                    format!("Failed to create target directory: {}", dir.display())
                })?;
        }

        dir.canonicalize()
            .map_err(|e| eyre::eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to canonicalize target directory: {}", dir.display())
            })?
    } else {
        std::env::current_dir()
            .map_err(|e| eyre::eyre!(e))
            .wrap_err("Failed to get current directory")?
    };

    verify_target_dir(&target_dir)?;

    let ui = UserInterface::new();

    apply_snapshot(&ui, &snapshot, &target_dir)
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| format!("Failed to apply snapshot from: {}", snapshot.display()))?;

    Ok(())
}

fn verify_target_dir(dir: &Path) -> eyre::Result<()> {
    // If path is a file, return error
    if dir.exists() && !dir.is_dir() {
        return Err(eyre::eyre!("Target path is a file: {}", dir.display()));
    }

    // Check if the directory is empty
    if dir
        .read_dir()
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| format!("Failed to read target directory: {}", dir.display()))?
        .next()
        .is_some()
    {
        return Err(eyre::eyre!(
            "Target directory is not empty: {}",
            dir.display()
        ));
    }

    Ok(())
}
