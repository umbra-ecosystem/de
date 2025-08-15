use crate::{
    project::Project,
    types::Slug,
    utils::{theme::Theme, ui::UserInterface},
    workspace::{Workspace, WorkspaceProject},
};
use console::style;
use eyre::{WrapErr, eyre};
use std::path::Path;
use std::process::Command;

/// Show the status of the active workspace and its projects.
pub fn status(workspace_name: Option<Slug>) -> eyre::Result<()> {
    tracing::info!("Starting status command");
    let ui = UserInterface::new();

    let workspace = if let Some(workspace_name) = workspace_name {
        tracing::info!("Loading workspace '{}'", workspace_name);
        Workspace::load_from_name(&workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to load workspace {workspace_name}"))?
            .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?
    } else {
        match Workspace::active()? {
            Some(ws) => ws,
            None => {
                ui.warning_item("No active workspace found.", None)?;
                return Ok(());
            }
        }
    };

    workspace_status(&ui, &workspace)?;

    tracing::info!("Finished status command");
    Ok(())
}

pub fn workspace_status(
    ui: &UserInterface,
    workspace: &Workspace,
) -> eyre::Result<WorkspaceStatus> {
    let ws_config = workspace.config();
    tracing::info!("Loaded workspace '{}'", ws_config.name);
    ui.heading(&format!("Workspace: {}", ws_config.name))?;
    ui.new_line()?; // Add a newline after the heading
    ui.heading("Projects:")?;
    let loading_bar = ui.loading_bar("Gathering project statuses...")?;

    let current_project = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load current project")?;

    let statuses: Vec<ProjectStatus> = ws_config
        .projects
        .iter()
        .map(|(project_name, ws_project)| {
            tracing::info!("Processing project '{}'", project_name);
            ProjectStatus::gather(
                &ws_config.name,
                project_name,
                ws_project,
                current_project.as_ref(),
            )
        })
        .collect();

    loading_bar.finish_and_clear();

    for status in &statuses {
        status.print(ui)?;
    }

    print_status_summary(ui, &statuses)?;

    Ok(WorkspaceStatus { statuses })
}

pub struct WorkspaceStatus {
    statuses: Vec<ProjectStatus>,
}

impl WorkspaceStatus {
    pub fn has_uncommited_or_unpushed_changes(&self) -> bool {
        self.statuses
            .iter()
            .any(|s| s.git.is_repo && (s.git.dirty || s.git.ahead.unwrap_or(0) > 0))
    }
}

/// Holds all dynamic status info for a project in the workspace.
struct ProjectStatus {
    slug: Slug,
    present: bool,
    current: bool,
    docker_services: Option<Vec<DockerServiceStatus>>,
    downed_services: Option<Vec<String>>,
    git: GitStatus,
}

/// Status for a single Docker Compose service.
struct DockerServiceStatus {
    name: String,
    status: String,
    ports: Option<String>,
}

impl ProjectStatus {
    /// Gather dynamic status for a project, including Git and Docker Compose state.
    fn gather(
        workspace_name: &Slug,
        project_name: &Slug,
        ws_project: &WorkspaceProject,
        current_project: Option<&Project>,
    ) -> Self {
        let dir = &ws_project.dir;
        let present = dir.exists();

        if !present {
            tracing::warn!(
                "Project '{}' directory missing: {}",
                project_name,
                dir.display()
            );
            return ProjectStatus {
                slug: project_name.clone(),
                present: false,
                current: false,
                docker_services: None,
                downed_services: None,
                git: GitStatus::not_repo(),
            };
        }

        match Project::from_dir(dir) {
            Ok(project) => {
                tracing::debug!("Loaded project manifest for '{}'", project_name);

                let current = current_project.as_ref().is_some_and(|p| {
                    &p.manifest().project().workspace == workspace_name
                        && &p.manifest().project().name == project_name
                });

                let dc_path = project.docker_compose_path().unwrap_or(None);
                let docker_services = dc_path.as_ref().and_then(|compose_path| {
                    tracing::debug!("Checking Docker Compose services for '{}'", project_name);
                    get_docker_services(compose_path)
                });
                let downed_services = dc_path
                    .as_ref()
                    .and_then(|compose_path| get_downed_services(compose_path));

                let git = if project.manifest().git.clone().unwrap_or_default().enabled {
                    GitStatus::gather(dir)
                } else {
                    GitStatus::disabled()
                };

                ProjectStatus {
                    slug: project_name.clone(),
                    present: true,
                    current,
                    docker_services,
                    downed_services,
                    git,
                }
            }
            Err(e) => {
                tracing::error!("Failed to load project '{}': {:?}", project_name, e);
                ProjectStatus {
                    slug: project_name.clone(),
                    present: true,
                    current: false,
                    docker_services: None,
                    downed_services: None,
                    git: GitStatus::not_repo(),
                }
            }
        }
    }

    /// Print the status for this project.
    fn print(&self, ui: &UserInterface) -> eyre::Result<()> {
        let theme = Theme::new();
        ui.writeln(&format!(
            "{} [{}] {}",
            style(&self.slug).bold(),
            if self.present {
                style("present").fg(theme.success_color)
            } else {
                style("missing").fg(theme.error_color)
            },
            if self.current {
                style("(current)").fg(theme.success_color).to_string()
            } else {
                "".to_string()
            }
        ))?;

        ui.indented(|ui| {
            ui.writeln(&format!("Git: {}", self.git.format(ui)))?;

            let Some(docker_services) = self.docker_services.as_ref() else {
                return Ok(());
            };

            if docker_services.is_empty() {
                return Ok(());
            }

            ui.writeln("Docker Compose services:")?;
            ui.indented(|ui| {
                for svc in docker_services {
                    let status_style = if svc.status.contains("Up") {
                        style(&svc.status).fg(theme.success_color)
                    } else {
                        style(&svc.status).fg(theme.error_color)
                    };

                    if let Some(ref ports) = svc.ports {
                        ui.writeln(&format!(
                            "{}: {} {}",
                            style(&svc.name).bold(),
                            status_style,
                            theme.dim(ports),
                        ))?;
                    } else {
                        ui.writeln(&format!("{}: {}", style(&svc.name).bold(), status_style))?;
                    }
                }

                Ok(())
            })?;

            Ok(())
        })?;

        Ok(())
    }
}

/// Git status for a project.
struct GitStatus {
    is_repo: bool,
    git_disabled: bool,
    branch: Option<String>,
    ahead: Option<u32>,
    behind: Option<u32>,
    dirty: bool,
}

/// Print a concise, actionable summary of project and service status.
fn print_status_summary(ui: &UserInterface, statuses: &[ProjectStatus]) -> eyre::Result<()> {
    let theme = Theme::new();
    let dirty = statuses
        .iter()
        .filter(|s| s.git.is_repo && s.git.dirty)
        .count();
    let ahead = statuses
        .iter()
        .filter(|s| s.git.is_repo && s.git.ahead.unwrap_or(0) > 0)
        .count();
    let behind = statuses
        .iter()
        .filter(|s| s.git.is_repo && s.git.behind.unwrap_or(0) > 0)
        .count();
    let downed_services_total: usize = statuses
        .iter()
        .filter_map(|s| s.downed_services.as_ref())
        .map(|downed| downed.len())
        .sum();

    tracing::info!(
        "Summary: dirty={}, ahead={}, behind={}, downed_services={}",
        dirty,
        ahead,
        behind,
        downed_services_total
    );

    println!();
    ui.heading("Status Summary:")?;
    let mut any = false;
    if dirty > 0 {
        ui.warning_item(
            &format!(
                "Uncommitted changes: {} (run: {})",
                dirty,
                style("git commit").fg(theme.accent_color)
            ),
            None,
        )?;
        any = true;
    }
    if behind > 0 {
        ui.info_item(&format!(
            "To pull: {} (run: {})",
            behind,
            style("git pull").fg(theme.accent_color)
        ))?;
        any = true;
    }
    if ahead > 0 {
        ui.info_item(&format!(
            "To push: {} (run: {})",
            ahead,
            style("git push").fg(theme.accent_color)
        ))?;
        any = true;
    }
    if downed_services_total > 0 {
        ui.error_item(
            &format!(
                "Downed services: {} (run: {})",
                downed_services_total,
                style("docker-compose up -d").fg(theme.accent_color)
            ),
            None,
        )?;
        any = true;
    }

    if !any {
        ui.success_item(
            &format!(
                "{}",
                style("All projects and services are up to date.").fg(theme.success_color)
            ),
            None,
        )?;
    }
    Ok(())
}

impl GitStatus {
    fn not_repo() -> Self {
        GitStatus {
            is_repo: false,
            git_disabled: false,
            branch: None,
            ahead: None,
            behind: None,
            dirty: false,
        }
    }

    fn disabled() -> Self {
        GitStatus {
            is_repo: false,
            git_disabled: true,
            branch: None,
            ahead: None,
            behind: None,
            dirty: false,
        }
    }

    fn gather(dir: &Path) -> Self {
        let git_dir = dir.join(".git");
        if !(git_dir.exists() && git_dir.is_dir()) {
            return GitStatus::not_repo();
        }

        let branch = Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("HEAD")
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            });

        let dirty = Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("status")
            .arg("--porcelain")
            .output()
            .ok()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);

        let ahead_behind = Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("status")
            .arg("-sb")
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let line = String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    Some(line)
                } else {
                    None
                }
            });

        let mut ahead = None;
        let mut behind = None;
        if let Some(ref ab) = ahead_behind
            && let Some(idx) = ab.find("[") {
                let ab_part = &ab[idx..];
                if let Some(a_idx) = ab_part.find("ahead ") {
                    let rest = &ab_part[a_idx + 6..];
                    if let Some(end) = rest.find(|c: char| !c.is_ascii_digit()) {
                        ahead = rest[..end].parse::<u32>().ok();
                    } else {
                        ahead = rest.parse::<u32>().ok();
                    }
                }
                if let Some(b_idx) = ab_part.find("behind ") {
                    let rest = &ab_part[b_idx + 7..];
                    if let Some(end) = rest.find(|c: char| !c.is_ascii_digit()) {
                        behind = rest[..end].parse::<u32>().ok();
                    } else {
                        behind = rest.parse::<u32>().ok();
                    }
                }
            }

        GitStatus {
            is_repo: true,
            git_disabled: false,
            branch,
            ahead,
            behind,
            dirty,
        }
    }

    fn format(&self, ui: &UserInterface) -> String {
        let theme = &ui.theme;

        if self.git_disabled {
            return theme.dim("git disabled");
        }

        if !self.is_repo {
            return theme.dim("not a git repo");
        }

        let mut out = String::new();

        if let Some(branch) = &self.branch {
            out.push_str(&ui.theme.highlight(branch));
        } else {
            out.push('?');
        }

        if let Some(a) = self.ahead {
            out.push_str(&format!(" (ahead {a})"));
        }

        if let Some(b) = self.behind {
            out.push_str(&format!(" (behind {b})"));
        }

        if self.dirty {
            out.push_str(&format!(", {}", theme.warn("dirty")));
        } else {
            out.push_str(&format!(", {}", theme.success("clean")));
        }

        out
    }
}

/// Get the status of all Docker Compose services for a project.
/// Returns a vector of DockerServiceStatus, or None if docker-compose fails.
fn get_docker_services(compose_path: &Path) -> Option<Vec<DockerServiceStatus>> {
    use std::process::Command;
    tracing::debug!("Running docker-compose ps -a for {:?}", compose_path);
    let output = Command::new("docker-compose")
        .arg("-f")
        .arg(compose_path)
        .arg("ps")
        .arg("-a")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut services = Vec::new();

    let mut lines = stdout.lines();
    let header_line = lines.next().unwrap_or("");
    let header_cols: Vec<&str> = header_line.split_whitespace().collect();
    let service_idx = header_cols.iter().position(|&h| h == "SERVICE");
    let status_idx = header_cols.iter().position(|&h| h == "STATUS");
    let ports_idx = header_cols.iter().position(|&h| h == "PORTS");

    if let (Some(service_idx), Some(status_idx)) = (service_idx, status_idx) {
        for line in lines {
            // Split line into columns by whitespace, but preserve spaces in STATUS and PORTS
            // We'll do this by splitting the line into fields based on header column positions
            let mut start_indices = Vec::new();
            let mut idx = 0;
            for col in &header_cols {
                // Find the start index of each column in the header line
                if let Some(pos) = header_line[idx..].find(col) {
                    start_indices.push(idx + pos);
                    idx += pos + col.len();
                }
            }
            // Now, for each column, extract the substring from the line
            let mut fields = Vec::new();
            for i in 0..start_indices.len() {
                let start = start_indices[i];
                let end = if i + 1 < start_indices.len() {
                    start_indices[i + 1]
                } else {
                    line.len()
                };
                let field = line.get(start..end).unwrap_or("").trim();
                fields.push(field);
            }
            // Now extract by header index
            if fields.len() <= status_idx {
                continue;
            }
            let name = fields[service_idx].to_string();
            let status = fields[status_idx].to_string();
            let ports = ports_idx
                .and_then(|idx| fields.get(idx).map(|s| s.to_string()))
                .filter(|s| !s.is_empty());
            tracing::debug!(
                "Service '{}' status: '{}', ports: {:?}",
                name,
                status,
                ports
            );
            services.push(DockerServiceStatus {
                name,
                status,
                ports,
            });
        }
    }

    Some(services)
}

fn get_downed_services(compose_path: &Path) -> Option<Vec<String>> {
    get_docker_services(compose_path).map(|services| {
        services
            .into_iter()
            .filter(|svc| !svc.status.contains("Up"))
            .map(|svc| svc.name)
            .collect()
    })
}
