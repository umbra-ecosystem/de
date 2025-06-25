use axoupdater::AxoUpdater;
use eyre::{WrapErr, eyre};

use crate::constants::PROJECT_NAME;

pub fn update() -> eyre::Result<()> {
    let mut updater = AxoUpdater::new_for(PROJECT_NAME);
    updater
        .load_receipt()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to initialize updater.")?;

    let updated = updater
        .run_sync()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to run updater")?;

    if let Some(updated) = updated {
        println!("Updated to version: {}", updated.new_version_tag);
    } else {
        println!("No updates available.");
    }

    Ok(())
}
