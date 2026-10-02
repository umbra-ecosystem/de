use eyre::{Context, eyre};

use crate::{
    project::Project,
    utils::{formatter::Formatter, theme::Theme},
    workspace::Workspace,
};

use crate::types::Slug;

pub fn info(workspace_name: Option<Slug>) -> eyre::Result<()> {
    let workspace = if let Some(workspace_name) = workspace_name {
        crate::workspace::registry::load_workspace(&workspace_name).map_err(|e| eyre!(e))?
    } else {
        Workspace::active()
            .wrap_err("Failed to get active workspace")?
            .ok_or_else(|| eyre!("No workspace is open. Run `de workspace select <name>` to open one; `de workspace list` shows the saved ones."))?
    };

    let theme = Theme::new();
    let formatter = Formatter::new();

    formatter.heading(&format!(
        "Workspace: {}",
        theme.highlight(workspace.config().name.as_str())
    ))?;

    if let Some(path) = workspace.config_path.to_str() {
        formatter.line(&format!("Path: {path}"), 2)?;
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
        formatter.info(&format!("{name}: {command}"))?;
    }

    Ok(())
}
