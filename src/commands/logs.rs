use eyre::{Context, Result, eyre};
use std::process::Command;

use crate::{project::Project, types::Slug, utils::ui::UserInterface, workspace::Workspace};

pub fn logs(
    workspace_name: Option<Slug>,
    service: Option<String>,
    follow: bool,
    tail: Option<usize>,
    extra_args: Vec<String>,
) -> Result<()> {
    let ui = UserInterface::new();

    // Try workspace mode first
    if let Some(ws_name) = workspace_name {
        let workspace = Workspace::load_from_name(&ws_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace '{}' not found", ws_name))?;

        ui.writeln(&format!(
            "Showing logs for workspace: {}",
            ui.theme.highlight(&workspace.config().name.to_string())
        ))?;
        ui.new_line()?;

        for (project_name, ws_project) in workspace.config().projects.iter() {
            let project = Project::from_dir(&ws_project.dir)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to load project '{}'", project_name))?;

            if let Some(compose_path) = project.docker_compose_path()? {
                ui.info_item(&format!(
                    "Logs for project: {}",
                    ui.theme.highlight(&project_name.to_string())
                ))?;
                show_logs(&compose_path, service.clone(), follow, tail, &extra_args)?;
                ui.new_line()?;
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
        ui.info_item("Running in no-config mode (no de.toml found)")?;
    }

    let compose_path = project
        .docker_compose_path()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get docker compose path")?
        .ok_or_else(|| eyre!("No docker-compose file found. Searched for:\n  - compose.yaml\n  - compose.yml\n  - docker-compose.yaml\n  - docker-compose.yml"))?;

    show_logs(&compose_path, service, follow, tail, &extra_args)?;

    Ok(())
}

fn show_logs(
    compose_path: &std::path::Path,
    service: Option<String>,
    follow: bool,
    tail: Option<usize>,
    extra_args: &[String],
) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.args(["compose", "-f"]);
    cmd.arg(compose_path);
    cmd.arg("logs");

    if follow {
        cmd.arg("-f");
    }

    if let Some(n) = tail {
        cmd.args(["--tail", &n.to_string()]);
    }

    if let Some(svc) = service {
        cmd.arg(svc);
    }

    // Add any extra arguments passed through
    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }

    let status = cmd
        .status()
        .wrap_err("Failed to execute docker compose logs")?;

    if !status.success() {
        return Err(eyre!("Docker compose logs command failed"));
    }

    Ok(())
}
