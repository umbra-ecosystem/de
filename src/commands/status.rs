use crate::project::Project;
use crate::types::Slug;
use crate::workspace::{Workspace, WorkspaceProject};
use std::path::Path;
use std::process::Command;

/// Show the status of the active workspace and its projects.
pub fn status() -> eyre::Result<()> {
    tracing::info!("Starting status command");
    let workspace = match Workspace::active()? {
        Some(ws) => ws,
        None => {
            tracing::warn!("No active workspace found");
            println!("No active workspace found.");
            return Ok(());
        }
    };

    let ws_config = workspace.config();
    tracing::info!("Loaded workspace '{}'", ws_config.name);
    println!("Workspace: {}", ws_config.name);
    println!("Projects:");

    let statuses: Vec<ProjectStatus> = ws_config
        .projects
        .iter()
        .map(|(slug, ws_project)| {
            tracing::info!("Processing project '{}'", slug);
            ProjectStatus::gather(slug, ws_project)
        })
        .collect();

    for status in &statuses {
        status.print();
    }

    print_status_summary(&statuses);

    tracing::info!("Finished status command");
    Ok(())
}

/// Holds all dynamic status info for a project in the workspace.
struct ProjectStatus {
    slug: Slug,
    present: bool,
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
    fn gather(slug: &Slug, ws_project: &WorkspaceProject) -> Self {
        let dir = &ws_project.dir;
        let present = dir.exists();

        if !present {
            tracing::warn!("Project '{}' directory missing: {}", slug, dir.display());
            return ProjectStatus {
                slug: slug.clone(),
                present: false,
                docker_services: None,
                downed_services: None,
                git: GitStatus::not_repo(),
            };
        }

        match Project::from_dir(dir) {
            Ok(project) => {
                tracing::debug!("Loaded project manifest for '{}'", slug);
                let dc_path = project.docker_compose_path().unwrap_or(None);
                let docker_services = dc_path.as_ref().and_then(|compose_path| {
                    tracing::debug!("Checking Docker Compose services for '{}'", slug);
                    get_docker_services(compose_path)
                });
                let downed_services = dc_path
                    .as_ref()
                    .and_then(|compose_path| get_downed_services(compose_path));
                let git = GitStatus::gather(dir);

                ProjectStatus {
                    slug: slug.clone(),
                    present: true,
                    docker_services,
                    downed_services,
                    git,
                }
            }
            Err(e) => {
                tracing::error!("Failed to load project '{}': {:?}", slug, e);
                ProjectStatus {
                    slug: slug.clone(),
                    present: true,
                    docker_services: None,
                    downed_services: None,
                    git: GitStatus::not_repo(),
                }
            }
        }
    }

    /// Print the status for this project.
    fn print(&self) {
        println!(
            "  - {} [{}]",
            self.slug,
            if self.present { "present" } else { "missing" }
        );
        println!("      Git: {}", self.git.format());
        if let Some(ref docker_services) = self.docker_services {
            if !docker_services.is_empty() {
                println!("      Docker Compose services:");
                for svc in docker_services {
                    if let Some(ref ports) = svc.ports {
                        println!("        {}: {} {}", svc.name, svc.status, ports);
                    } else {
                        println!("        {}: {}", svc.name, svc.status);
                    }
                }
            }
        }
    }
}

/// Git status for a project.
struct GitStatus {
    is_repo: bool,
    branch: Option<String>,
    ahead: Option<u32>,
    behind: Option<u32>,
    dirty: bool,
}

/// Print a concise, actionable summary of project and service status.
/// Only nonzero items are shown, each with a one-line suggestion.
/// If everything is clean, prints a single "All projects and services are up to date."
fn print_status_summary(statuses: &[ProjectStatus]) {
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
    println!("Status Summary:");
    let mut any = false;
    if dirty > 0 {
        println!("  Uncommitted changes: {}    (run: git commit)", dirty);
        any = true;
    }
    if behind > 0 {
        println!("  To pull: {}                (run: git pull)", behind);
        any = true;
    }
    if ahead > 0 {
        println!("  To push: {}                (run: git push)", ahead);
        any = true;
    }
    if downed_services_total > 0 {
        println!(
            "  Downed services: {}        (run: docker-compose up -d)",
            downed_services_total
        );
        any = true;
    }
    if !any {
        println!("  All projects and services are up to date.");
    }
}

impl GitStatus {
    fn not_repo() -> Self {
        GitStatus {
            is_repo: false,
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
        if let Some(ref ab) = ahead_behind {
            if let Some(idx) = ab.find("[") {
                let ab_part = &ab[idx..];
                if let Some(a_idx) = ab_part.find("ahead ") {
                    let rest = &ab_part[a_idx + 6..];
                    if let Some(end) = rest.find(|c: char| !c.is_digit(10)) {
                        ahead = rest[..end].parse::<u32>().ok();
                    } else {
                        ahead = rest.parse::<u32>().ok();
                    }
                }
                if let Some(b_idx) = ab_part.find("behind ") {
                    let rest = &ab_part[b_idx + 7..];
                    if let Some(end) = rest.find(|c: char| !c.is_digit(10)) {
                        behind = rest[..end].parse::<u32>().ok();
                    } else {
                        behind = rest.parse::<u32>().ok();
                    }
                }
            }
        }

        GitStatus {
            is_repo: true,
            branch,
            ahead,
            behind,
            dirty,
        }
    }

    fn format(&self) -> String {
        if !self.is_repo {
            return "not a git repo".to_string();
        }
        let mut out = String::new();
        if let Some(branch) = &self.branch {
            out.push_str(branch);
        } else {
            out.push('?');
        }
        if let Some(a) = self.ahead {
            out.push_str(&format!(" (ahead {})", a));
        }
        if let Some(b) = self.behind {
            out.push_str(&format!(" (behind {})", b));
        }
        if self.dirty {
            out.push_str(", dirty");
        } else {
            out.push_str(", clean");
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
