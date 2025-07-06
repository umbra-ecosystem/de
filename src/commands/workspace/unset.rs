use eyre::Context;

use crate::{config::Config, utils::theme::Theme};

pub fn unset() -> eyre::Result<()> {
    let config = Config::load()?;
    let active_workspace = config.get_active_workspace();

    let Some(active_workspace) = active_workspace else {
        println!("No active workspace to unset.");
        return Ok(());
    };

    Config::mutate_persisted(|config| {
        config.set_active_workspace(None);
    })
    .wrap_err("Failed to unset active workspace")?;

    let theme = Theme::new();

    let awc = theme.info(active_workspace.as_str());
    println!("Active workspace '{awc}' has been removed.");

    Ok(())
}
