use eyre::{Context, Result, eyre};
use std::process::Command;

use crate::{project::Project, types::Slug, utils::theme::Theme, workspace::Workspace};

pub fn ps(workspace_name: Option<Slug>, extra_args: Vec<String>) -> Result<()> {
    let theme = Theme::new();

    // Try workspace mode first
    if let Some(ws_name) = workspace_name {
        let workspace = Workspace::load_from_name(&ws_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace '{}' not found", ws_name))?;

        println!(
            "{} Showing containers for workspace: {}\n",
            console::style("ℹ").cyan(),
            theme.highlight(&workspace.config().name.to_string())
        );

        for (project_name, ws_project) in workspace.config().projects.iter() {
            let project = Project::from_dir(&ws_project.dir)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to load project '{}'", project_name))?;

            if let Some(compose_path) = project.docker_compose_path()? {
                println!(
                    "{} Containers for project: {}",
                    console::style("→").cyan(),
                    theme.highlight(&project_name.to_string())
                );
                show_ps(&compose_path, &extra_args)?;
                println!();
            }
        }

        return Ok(());
    }

    // Try current/inferred project (no-config mode)
    let project = Project::current_or_inferred()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to detect project")?
        .ok_or_else(|| eyre!("No project found in current directory"))?;

    if project.is_inferred() {
        println!(
            "{} Running in no-config mode (no de.toml found)",
            console::style("ℹ").cyan()
        );
    }

    let compose_path = project
        .docker_compose_path()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get docker compose path")?
        .ok_or_else(|| eyre!("No docker-compose file found. Searched for:\n  - compose.yaml\n  - compose.yml\n  - docker-compose.yaml\n  - docker-compose.yml"))?;

    show_ps(&compose_path, &extra_args)?;

    Ok(())
}

fn show_ps(compose_path: &std::path::Path, extra_args: &[String]) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.args(["compose", "-f"]);
    cmd.arg(compose_path);
    cmd.arg("ps");

    // Add any extra arguments passed through
    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }

    let status = cmd
        .status()
        .wrap_err("Failed to execute docker compose ps")?;

    if !status.success() {
        return Err(eyre!("Docker compose ps command failed"));
    }

    Ok(())
}
