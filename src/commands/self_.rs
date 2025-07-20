use axoupdater::AxoUpdater;
use eyre::{WrapErr, eyre};

use crate::{constants::PROJECT_NAME, utils::ui::UserInterface};

pub fn update() -> eyre::Result<()> {
    let ui = UserInterface::new();
    let loading_bar = ui.loading_bar("Checking for updates...")?;

    let mut updater = AxoUpdater::new_for(PROJECT_NAME);
    updater
        .load_receipt()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to initialize updater.")?;

    let is_update_needed = updater
        .is_update_needed_sync()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to check for updates")?;

    if is_update_needed {
        loading_bar.set_message("Update available, downloading...");
    } else {
        loading_bar.finish_and_clear();
        ui.writeln("You are already on the latest version.")?;
        return Ok(());
    }

    let updated = updater
        .disable_installer_output()
        .run_sync()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to run updater")?;

    loading_bar.finish_and_clear();

    if let Some(updated) = updated {
        ui.writeln(&format!(
            "Update successful! Version {} installed.",
            ui.theme.highlight(updated.new_version_tag.as_str()),
        ))?;
    } else {
        ui.writeln("You are already on the latest version.")?;
    }

    Ok(())
}
