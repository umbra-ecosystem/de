use std::path::PathBuf;

use crate::{
    project::Project,
    types::Slug,
    utils::{theme::Theme, ui::UserInterface},
    workspace::Workspace,
};
use console::style;
use eyre::{Context, eyre};

pub fn list(workspace: Workspace) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let theme = Theme::new();
    let name = &workspace.config().name;

    if workspace.config().projects.is_empty() {
        ui.warning_item(&format!("No projects found in workspace '{name}'"), None)?;
        return Ok(());
    }

    let current_project = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?;

    let mut projects_to_display: Vec<ProjectDisplay> = Vec::new();

    for (id, wp) in &workspace.config().projects {
        let project_dir = &wp.dir;
        let present = project_dir.exists();

        let project_name = if present {
            match Project::from_dir(project_dir) {
                Ok(project) => project.manifest().project().name.clone(),
                Err(_) => id.clone(), // Fallback if project manifest can't be loaded
            }
        } else {
            id.clone()
        };

        let current = if let Some(cp) = &current_project {
            cp.dir() == project_dir
        } else {
            false
        };

        projects_to_display.push(ProjectDisplay {
            id: id.clone(),
            name: project_name,
            dir: project_dir.clone(),
            present,
            current,
        });
    }

    projects_to_display.sort_by(|a, b| a.id.cmp(&b.id));

    ui.heading(&format!("Projects in workspace {name}:"))?;
    for project in &projects_to_display {
        print_project_display(project, &ui, &theme)?;
    }

    Ok(())
}

struct ProjectDisplay {
    id: Slug,
    name: Slug,
    dir: PathBuf,
    present: bool,
    current: bool,
}

fn print_project_display(
    project: &ProjectDisplay,
    ui: &UserInterface,
    theme: &Theme,
) -> eyre::Result<()> {
    let current_indicator = if project.current {
        format!(" {}", style("(current)").fg(theme.accent_color))
    } else {
        "".to_string()
    };

    if project.present {
        ui.success_item(&format!("{}{}", project.name, current_indicator), None)?;
    } else {
        ui.error_item(
            project.name.as_str(),
            Some(&format!(
                "Project directory '{}' does not exist",
                project.dir.display()
            )),
        )?;
    }

    Ok(())
}
