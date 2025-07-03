use crate::{
    project::Project, types::Slug, utils::formatter::Formatter, utils::theme::Theme,
    workspace::Workspace,
};
use console::style;
use eyre::{Context, eyre};

pub fn list(workspace: Workspace) -> eyre::Result<()> {
    let formatter = Formatter::new();
    let theme = Theme::new();
    let name = &workspace.config().name;

    if workspace.config().projects.is_empty() {
        formatter.warning(&format!("No projects found in workspace '{}'", name), None);
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
            present,
            current,
        });
    }

    projects_to_display.sort_by(|a, b| a.id.cmp(&b.id));

    formatter.heading(&format!("Projects in workspace {}:", name));
    for project in &projects_to_display {
        print_project_display(project, &formatter, &theme);
    }

    Ok(())
}

struct ProjectDisplay {
    id: Slug,
    name: Slug,
    present: bool,
    current: bool,
}

fn print_project_display(project: &ProjectDisplay, formatter: &Formatter, theme: &Theme) {
    let status_symbol = if project.present {
        formatter.success_symbol()
    } else {
        formatter.error_symbol()
    };

    let current_indicator = if project.current {
        format!(" {}", style("(current)").fg(theme.accent_color))
    } else {
        "".to_string()
    };

    println!(
        "  {} {}{}",
        status_symbol,
        style(&project.name).bold(),
        current_indicator
    );
}
