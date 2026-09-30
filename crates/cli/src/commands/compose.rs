use eyre::{Context, eyre};

use crate::{
    project::Project,
    types::Slug,
    utils::{get_workspace_for_cli, ui::UserInterface},
    workspace::{Order, ordered_projects},
};

pub fn compose(
    project_name: Option<Slug>,
    workspace_name: Option<Option<Slug>>,
    args: Vec<String>,
) -> eyre::Result<()> {
    let ui = UserInterface::new();

    if let Some(workspace_name) = workspace_name {
        let workspace = get_workspace_for_cli(Some(workspace_name))?;

        for (name, project) in ordered_projects(&workspace, Order::Startup)? {
            ui.info_item(&format!("Project: {}", ui.theme.highlight(name.as_str())))?;

            if !project.compose(&args)? {
                ui.warning_item("No docker compose file found", None)?;
            }
            ui.new_line()?;
        }

        return Ok(());
    }

    let project = match project_name {
        Some(name) => crate::utils::get_project_for_cli(Some(name), Some(None))?,
        None => Project::current_or_inferred()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to detect project")?
            .ok_or_else(|| eyre!("No project found in current directory"))?,
    };

    if !project.compose(&args)? {
        return Err(eyre!(
            "No docker compose file found. Searched for compose.yaml, compose.yml, docker-compose.yaml, docker-compose.yml"
        ));
    }

    Ok(())
}
