//! `de workspace select <name>`: make a workspace the active one, then report its services.
//!
//! Selecting validates the name against the saved workspaces first — a name that is not saved
//! can never reach `config.toml` — and never starts or stops anything; `de start -w <name>` is
//! still the way to run the services.

use eyre::eyre;

use super::status::{now, report};
use crate::{
    types::Slug,
    utils::ui::UserInterface,
    workspace::{health, registry},
};

pub fn select(name: Slug) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let now = now()?;

    // The errors are the registry's own: they name the file or the command to run next.
    let workspace = registry::select_workspace(&name, now).map_err(|e| eyre!(e))?;

    ui.success_item(
        &format!("Switched to workspace: {}", ui.theme.highlight(name.as_str())),
        None,
    )?;
    ui.new_line()?;

    let health = health::check_workspace(&workspace, now);
    report(&ui, &health)
}
