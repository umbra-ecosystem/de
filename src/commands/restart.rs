use eyre::{Context, Result, eyre};
use std::process::Command;

use crate::{project::Project, types::Slug, utils::theme::Theme, workspace::Workspace};

pub fn restart(
    workspace_name: Option<Slug>,
    service: Option<String>,
    extra_args: Vec<String>,
) -> Result<()> {
    let theme = Theme::new();

    // Try workspace mode first
    if let Some(ws_name) = workspace_name {
        let workspace = Workspace::load_from_name(&ws_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace '{}' not found", ws_name))?;

        println!(
            "{} Restarting services for workspace: {}\n",
            console::style("ℹ").cyan(),
            theme.highlight(&workspace.config().name.to_string())
        );

        for (project_name, ws_project) in workspace.config().projects.iter() {
            let project = Project::from_dir(&ws_project.dir)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to load project '{}'", project_name))?;

            if let Some(compose_path) = project.docker_compose_path()? {
                println!(
                    "{} Restarting project: {}",
                    console::style("→").cyan(),
                    theme.highlight(&project_name.to_string())
                );
                restart_services(&compose_path, service.clone(), &extra_args)?;
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

    restart_services(&compose_path, service, &extra_args)?;

    Ok(())
}

fn restart_services(
    compose_path: &std::path::Path,
    service: Option<String>,
    extra_args: &[String],
) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.args(["compose", "-f"]);
    cmd.arg(compose_path);
    cmd.arg("restart");

    if let Some(svc) = service {
        cmd.arg(svc);
    }

    // Add any extra arguments passed through
    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }

    let status = cmd
        .status()
        .wrap_err("Failed to execute docker compose restart")?;

    if !status.success() {
        return Err(eyre!("Docker compose restart command failed"));
    }

    Ok(())
}
