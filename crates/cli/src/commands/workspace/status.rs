//! `de workspace status`: what the services of a workspace are doing.
//!
//! The report itself ([`report`]) is what `de workspace select` and `de start` print after they
//! run, so one workspace reads the same everywhere. The rows are built by the pure
//! [`row_line`], which is what the tests cover.

use std::time::{SystemTime, UNIX_EPOCH};

use eyre::{Context, eyre};

use crate::{
    types::Slug,
    utils::ui::UserInterface,
    workspace::{
        Selected, Workspace,
        health::{self, Overall, ProjectHealth, ProjectStatus, ServiceHealth, WorkspaceHealth},
        registry,
    },
};

pub fn status(workspace: Option<Slug>) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let workspace = resolve(workspace)?;
    let health = health::check_workspace(&workspace, now()?);
    report(&ui, &health)
}

/// Which workspace the status is about: the `-w` flag, else this folder's workspace, else the
/// selected one — with a message that says what to run when there is none.
pub fn resolve(workspace: Option<Slug>) -> eyre::Result<Workspace> {
    if let Some(name) = workspace {
        return registry::load_workspace(&name).map_err(|e| eyre!(e));
    }

    // This folder's workspace first, like every other command resolves it.
    if let Some(workspace) = Workspace::current()
        .map_err(|e| eyre!(e))
        .wrap_err("The workspace of this folder could not be read; run `de workspace list` to see which workspace files are readable")?
    {
        return Ok(workspace);
    }

    match registry::selected_workspace()? {
        Selected::Workspace(workspace) => Ok(workspace),
        Selected::Missing(name) => Err(eyre!(
            "The workspace '{name}' is no longer saved. \
             Run `de workspace list` to see the saved ones, then `de workspace select <name>`."
        )),
        Selected::None => Err(eyre!(
            "No workspace is open. Run `de workspace select <name>` to open one; \
             `de workspace list` shows the saved ones."
        )),
    }
}

/// What one project's line says about its services, and how loudly.
///
/// The defaults (all running, no services here) stay quiet; only what is out of the ordinary
/// gets a tone of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Ok,
    Warn,
    Bad,
    /// Nothing to report about this project (it has no services).
    Mute,
}

pub(crate) fn row_line(project: &ProjectStatus) -> (Tone, String) {
    let name = project.project.as_str();
    match &project.health {
        ProjectHealth::MissingFolder => (
            Tone::Bad,
            format!("{name} — the folder {} is missing", project.dir.display()),
        ),
        ProjectHealth::NoCompose => (Tone::Mute, format!("{name} — no services")),
        ProjectHealth::Unavailable(reason) => (Tone::Bad, format!("{name} — {reason}")),
        ProjectHealth::Services(services) if services.is_empty() => {
            (Tone::Mute, format!("{name} — no services"))
        }
        ProjectHealth::Services(services) => {
            let declared = services.len();
            let running = services.iter().filter(|s| s.state.is_running()).count();
            let unhealthy: Vec<&str> = services
                .iter()
                .filter(|s| s.health == ServiceHealth::Unhealthy)
                .map(|s| s.service.as_str())
                .collect();

            if !unhealthy.is_empty() {
                return (
                    Tone::Bad,
                    format!(
                        "{name} — {running} of {declared} running, {} unhealthy",
                        unhealthy.join(", ")
                    ),
                );
            }
            if running == declared {
                (Tone::Ok, format!("{name} — {declared} running"))
            } else if running == 0 {
                (Tone::Warn, format!("{name} — stopped ({declared})"))
            } else {
                (Tone::Warn, format!("{name} — {running} of {declared} running"))
            }
        }
    }
}

/// Print a workspace's services, project by project, with one line saying where it stands.
pub fn report(ui: &UserInterface, health: &WorkspaceHealth) -> eyre::Result<()> {
    if health.projects.is_empty() {
        ui.info_item(
            "This workspace has no projects. Run `de init` in a project folder to add one.",
        )?;
        return Ok(());
    }

    // Docker itself could not be asked: every project would repeat the same reason, so the
    // reason is the report.
    if health.overall() == Overall::Unavailable {
        ui.error_item(&health.describe(), None)?;
        return Ok(());
    }

    ui.heading("Services")?;
    for project in &health.projects {
        let (tone, line) = row_line(project);
        match tone {
            Tone::Ok => ui.success_item(&line, None)?,
            Tone::Warn => ui.warning_item(&line, None)?,
            Tone::Bad => ui.error_item(&line, None)?,
            Tone::Mute => ui.writeln(&ui.theme.dim(&line))?,
        }
    }

    ui.new_line()?;
    let summary = health.describe();
    match health.overall() {
        Overall::Up => ui.writeln(&ui.theme.success(&summary))?,
        Overall::Unavailable => ui.writeln(&ui.theme.error(&summary))?,
        Overall::Partial | Overall::Down if health.unhealthy() > 0 => {
            ui.writeln(&ui.theme.error(&summary))?;
        }
        Overall::Partial | Overall::Down => ui.writeln(&ui.theme.warn(&summary))?,
        Overall::Empty => ui.writeln(&ui.theme.dim(&summary))?,
    }
    Ok(())
}

pub(crate) fn now() -> eyre::Result<i64> {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .wrap_err("The system clock is before 1970")?;
    Ok(since_epoch.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::workspace::health::{ContainerState, DockerError, ServiceStatus, ProjectStatus};

    fn slug(s: &str) -> Slug {
        s.parse().unwrap()
    }

    fn status_of(health: ProjectHealth) -> ProjectStatus {
        ProjectStatus {
            project: slug("api"),
            dir: PathBuf::from("/w/api"),
            health,
        }
    }

    fn service(name: &str, state: ContainerState, health: ServiceHealth) -> ServiceStatus {
        ServiceStatus {
            service: name.into(),
            state,
            health,
            exit_code: None,
        }
    }

    #[test]
    fn all_running_says_little_and_a_folder_that_is_gone_says_where() {
        let (tone, line) = row_line(&status_of(ProjectHealth::Services(vec![
            service("api", ContainerState::Running, ServiceHealth::Healthy),
            service("worker", ContainerState::Running, ServiceHealth::NoCheck),
        ])));
        assert_eq!((tone, line.as_str()), (Tone::Ok, "api — 2 running"));

        let (tone, line) = row_line(&status_of(ProjectHealth::MissingFolder));
        assert_eq!(tone, Tone::Bad);
        assert!(line.contains("the folder /w/api is missing"), "{line}");

        let (tone, line) = row_line(&status_of(ProjectHealth::NoCompose));
        assert_eq!((tone, line.as_str()), (Tone::Mute, "api — no services"));
    }

    #[test]
    fn stopped_and_partial_are_warnings_and_an_unhealthy_service_is_named() {
        let (tone, line) = row_line(&status_of(ProjectHealth::Services(vec![
            service("api", ContainerState::Exited, ServiceHealth::NoCheck),
            service("worker", ContainerState::Created, ServiceHealth::NoCheck),
        ])));
        assert_eq!((tone, line.as_str()), (Tone::Warn, "api — stopped (2)"));

        let (tone, line) = row_line(&status_of(ProjectHealth::Services(vec![
            service("api", ContainerState::Running, ServiceHealth::Healthy),
            service("worker", ContainerState::Exited, ServiceHealth::NoCheck),
        ])));
        assert_eq!((tone, line.as_str()), (Tone::Warn, "api — 1 of 2 running"));

        let (tone, line) = row_line(&status_of(ProjectHealth::Services(vec![
            service("api", ContainerState::Running, ServiceHealth::Unhealthy),
            service("worker", ContainerState::Running, ServiceHealth::Healthy),
        ])));
        assert_eq!(tone, Tone::Bad);
        assert_eq!(line, "api — 2 of 2 running, api unhealthy");
    }

    #[test]
    fn a_docker_problem_shows_the_reason_it_was_written_with() {
        let (tone, line) = row_line(&status_of(ProjectHealth::Unavailable(DockerError::DaemonDown)));
        assert_eq!(tone, Tone::Bad);
        assert!(line.contains("Start Docker Desktop"), "{line}");
    }
}
