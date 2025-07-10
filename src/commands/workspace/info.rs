use eyre::{Context, eyre};

use crate::{
    project::Project,
    utils::{formatter::Formatter, theme::Theme},
    workspace::Workspace,
};

use crate::types::Slug;

pub fn info(workspace_name: Option<Slug>) -> eyre::Result<()> {
    let workspace = if let Some(workspace_name) = workspace_name {
        Workspace::load_from_name(&workspace_name)
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace '{}' not found", workspace_name))?
    } else {
        Workspace::active()
            .wrap_err("Failed to get active workspace")?
            .ok_or_else(|| eyre!("No active workspace found"))?
    };

    let theme = Theme::new();
    let formatter = Formatter::new();

    formatter.heading(&format!(
        "Workspace: {}",
        theme.highlight(workspace.config().name.as_str())
    ))?;

    if let Some(path) = workspace.config_path.to_str() {
        formatter.line(&format!("Path: {}", path), 2)?;
    }

    formatter.new_line()?;
    formatter.heading(&format!("Projects: {}", workspace.config().projects.len()))?;
    for (name, project) in workspace.config().projects.iter() {
        let is_valid = Project::from_dir(&project.dir).is_ok();
        if is_valid {
            formatter.success(&format!("{}: {}", name, project.dir.display()))?;
        } else {
            formatter.error(
                &format!("{}: {}", name, project.dir.display()),
                Some("Invalid project directory or missing manifest"),
            )?;
        }
    }

    formatter.new_line()?;
    formatter.heading(&format!("Tasks: {}", workspace.config().tasks.len()))?;
    for (name, command) in workspace.config().tasks.iter() {
        formatter.info(&format!("{}: {}", name, command))?;
    }

    Ok(())
}
