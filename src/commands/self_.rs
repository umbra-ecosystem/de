use crate::commands;
use axoupdater::{AxoUpdater, UpdateResult};
use eyre::{WrapErr, eyre};
use indicatif::ProgressBar;

use crate::{constants::PROJECT_NAME, utils::ui::UserInterface};

pub fn update() -> eyre::Result<()> {
    let ui = UserInterface::new();
    let mut loading_bar = ui.loading_bar("Checking for updates...")?;

    if let Some(update_result) = update_binary(&ui, &mut loading_bar)? {
        commands::shim::reinstate()?;

        ui.new_line()?;
        ui.writeln(&format!(
            "Update successful! Version {} installed.",
            update_result.new_version_tag
        ))?;
    }

    loading_bar.finish_and_clear();

    Ok(())
}

fn update_binary(
    ui: &UserInterface,
    loading_bar: &mut ProgressBar,
) -> eyre::Result<Option<UpdateResult>> {
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
        return Ok(None);
    }

    let updated = updater
        .disable_installer_output()
        .run_sync()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to run updater")?;

    if let Some(updated) = updated {
        Ok(Some(updated))
    } else {
        ui.writeln("You are already on the latest version.")?;
        Ok(None)
    }
}
