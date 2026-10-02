//! Docker Compose status for a workspace: what its services are doing, in the terms a screen
//! or a report shows.
//!
//! The parsers here are pure functions over the JSON docker writes, so they are tested without
//! docker; only [`check_workspace`], [`running_workspaces`] and [`run_compose`] start a process.
//! Every reason a user reads is written on [`DockerError`], so the CLI and the window say the
//! same words.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::{Project, find_compose_file_in_dir};
use crate::types::Slug;
use crate::workspace::{Workspace, utils::startup_order};

/// Why docker could not answer, in words that say what to do about it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DockerError {
    #[error(
        "Docker is not installed, or not on your PATH. \
         Install Docker Desktop to run and check this workspace's services."
    )]
    NotInstalled,

    #[error("Docker is not running. Start Docker Desktop, then try again.")]
    DaemonDown,

    #[error(
        "Docker Compose is not installed with Docker. \
         Install the Compose plugin, then try again."
    )]
    ComposeMissing,

    #[error("Docker said: {0}")]
    Other(String),
}

/// What docker's complaint means. Pure, so the wording is tested without a daemon.
pub fn classify_docker_failure(text: &str) -> DockerError {
    let low = text.to_lowercase();

    const DAEMON_DOWN: &[&str] = &[
        "cannot connect to the docker daemon",
        "error during connect",
        "is the docker daemon running",
        "docker daemon is not running",
        "docker desktop is not running",
        "docker desktop is stopped",
    ];
    if DAEMON_DOWN.iter().any(|marker| low.contains(marker)) {
        return DockerError::DaemonDown;
    }

    const NO_COMPOSE: &[&str] = &[
        "is not a docker command",
        "unknown command: 'compose'",
        "no such command: compose",
    ];
    if NO_COMPOSE.iter().any(|marker| low.contains(marker)) {
        return DockerError::ComposeMissing;
    }

    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("it failed without saying why");
    DockerError::Other(first.to_string())
}

/// What one container is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    Running,
    Exited,
    Created,
    Paused,
    Pausing,
    Restarting,
    Removing,
    Dead,
    /// A declared service that has never had a container.
    Absent,
    /// A state this version of docker invented.
    Unknown,
}

impl ContainerState {
    pub fn from_docker(state: &str) -> Self {
        match state.trim().to_ascii_lowercase().as_str() {
            "running" => Self::Running,
            "exited" => Self::Exited,
            "created" => Self::Created,
            "paused" => Self::Paused,
            "pausing" => Self::Pausing,
            "restarting" => Self::Restarting,
            "removing" => Self::Removing,
            "dead" => Self::Dead,
            "" => Self::Absent,
            _ => Self::Unknown,
        }
    }

    pub fn is_running(self) -> bool {
        self == Self::Running
    }

    /// How the row reads on screen when it is not simply "running".
    pub fn describe(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited => "stopped",
            Self::Created => "created, not started",
            Self::Paused => "paused",
            Self::Pausing => "pausing",
            Self::Restarting => "restarting",
            Self::Removing => "removing",
            Self::Dead => "dead",
            Self::Absent => "never started",
            Self::Unknown => "unknown",
        }
    }
}

/// The healthcheck of a service, if it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServiceHealth {
    NoCheck,
    Healthy,
    Starting,
    Unhealthy,
}

impl ServiceHealth {
    pub fn from_docker(health: &str) -> Self {
        match health.trim().to_ascii_lowercase().as_str() {
            "healthy" => Self::Healthy,
            "unhealthy" => Self::Unhealthy,
            "starting" => Self::Starting,
            _ => Self::NoCheck,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::NoCheck => "",
            Self::Healthy => "healthy",
            Self::Starting => "starting",
            Self::Unhealthy => "unhealthy",
        }
    }
}

/// One service of one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub service: String,
    pub state: ContainerState,
    pub health: ServiceHealth,
    /// The exit code of a stopped container, when it has one.
    pub exit_code: Option<i32>,
}

impl ServiceStatus {
    /// The word for the row when it is not simply a running, healthy service.
    pub fn exception(&self) -> Option<String> {
        if self.health == ServiceHealth::Unhealthy {
            return Some("unhealthy".into());
        }
        if self.state.is_running() {
            return None;
        }
        Some(self.state.describe().into())
    }
}

/// Everything known about one project's services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectHealth {
    /// The project's folder is gone: it was moved, renamed or deleted.
    MissingFolder,
    /// The folder is there but has no compose file: nothing to run or check.
    NoCompose,
    /// Docker could not be asked (see the reason).
    Unavailable(DockerError),
    /// One entry per service, declared or still created.
    Services(Vec<ServiceStatus>),
}

impl ProjectHealth {
    /// How many services the project declares (or still has containers for).
    pub fn declared(&self) -> usize {
        match self {
            Self::Services(services) => services.len(),
            _ => 0,
        }
    }

    pub fn running(&self) -> usize {
        match self {
            Self::Services(services) => services.iter().filter(|s| s.state.is_running()).count(),
            _ => 0,
        }
    }

    pub fn unhealthy(&self) -> usize {
        match self {
            Self::Services(services) => services
                .iter()
                .filter(|s| s.health == ServiceHealth::Unhealthy)
                .count(),
            _ => 0,
        }
    }

    pub fn unavailable(&self) -> Option<&DockerError> {
        match self {
            Self::Unavailable(e) => Some(e),
            _ => None,
        }
    }

    pub fn missing_folder(&self) -> bool {
        matches!(self, Self::MissingFolder)
    }
}

/// One project of the workspace, as the health check found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectStatus {
    pub project: Slug,
    pub dir: PathBuf,
    pub health: ProjectHealth,
}

/// The health of a whole workspace, checked at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHealth {
    pub workspace: Slug,
    /// The projects in the order they start.
    pub projects: Vec<ProjectStatus>,
    /// When this was checked (unix seconds).
    pub checked_at: i64,
}

/// The one word for a workspace's services.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overall {
    /// Every declared service is running (and healthy).
    Up,
    /// Some are running, some are not.
    Partial,
    /// Services are declared but none is running.
    Down,
    /// Docker itself could not be asked.
    Unavailable,
    /// No project in this workspace has services at all.
    Empty,
}

impl WorkspaceHealth {
    pub fn overall(&self) -> Overall {
        if self.projects.iter().any(|p| p.health.unavailable().is_some()) {
            return Overall::Unavailable;
        }
        let declared: usize = self.declared();
        let running = self.running();
        if declared == 0 {
            return Overall::Empty;
        }
        if running == declared && self.unhealthy() == 0 {
            Overall::Up
        } else if running == 0 {
            Overall::Down
        } else {
            Overall::Partial
        }
    }

    pub fn declared(&self) -> usize {
        self.projects.iter().map(|p| p.health.declared()).sum()
    }

    pub fn running(&self) -> usize {
        self.projects.iter().map(|p| p.health.running()).sum()
    }

    pub fn unhealthy(&self) -> usize {
        self.projects.iter().map(|p| p.health.unhealthy()).sum()
    }

    /// At least one service is running: the dot a list of workspaces shows.
    pub fn any_running(&self) -> bool {
        self.projects.iter().any(|p| p.health.running() > 0)
    }

    /// The names of the unhealthy services, as `project/service`.
    pub fn unhealthy_services(&self) -> Vec<String> {
        self.projects
            .iter()
            .flat_map(|p| {
                match &p.health {
                    ProjectHealth::Services(services) => services
                        .iter()
                        .filter(|s| s.health == ServiceHealth::Unhealthy)
                        .map(|s| format!("{}/{}", p.project, s.service))
                        .collect(),
                    _ => Vec::new(),
                }
            })
            .collect()
    }

    /// One line saying something is off, or `None` when everything is fine (silence is the
    /// normal state; a badge means something is wrong).
    pub fn exception(&self) -> Option<String> {
        match self.overall() {
            Overall::Up | Overall::Empty => None,
            Overall::Unavailable => Some(self.unavailable_reason()),
            Overall::Down => Some("Services are stopped.".into()),
            Overall::Partial => {
                if self.unhealthy() > 0 {
                    return Some(self.unhealthy_line());
                }
                Some(format!(
                    "{} of {} services are running.",
                    self.running(),
                    self.declared()
                ))
            }
        }
    }

    /// A full line for a report, where saying things are fine is the point.
    pub fn describe(&self) -> String {
        match self.overall() {
            Overall::Up => format!(
                "All {} services are running across {} project{}.",
                self.declared(),
                self.projects.len(),
                if self.projects.len() == 1 { "" } else { "s" }
            ),
            Overall::Empty => "No services in this workspace.".into(),
            Overall::Unavailable => self.unavailable_reason(),
            Overall::Partial | Overall::Down => self
                .exception()
                .unwrap_or_else(|| "Everything is running.".into()),
        }
    }

    fn unavailable_reason(&self) -> String {
        self.projects
            .iter()
            .find_map(|p| p.health.unavailable())
            .map(ToString::to_string)
            .unwrap_or_else(|| "Docker could not be asked.".into())
    }

    fn unhealthy_line(&self) -> String {
        let names = self.unhealthy_services();
        let (one, many) = (names.len() == 1, names.len());
        let shown = names
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = if many > 2 {
            format!(" and {} more", many - 2)
        } else {
            String::new()
        };
        format!(
            "{shown}{more} {} unhealthy.",
            if one { "is" } else { "are" }
        )
    }
}

/// What `docker compose` would run for a project folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposePlan {
    /// The folder does not exist (any more).
    Missing,
    /// The folder exists but has no compose file.
    NoCompose,
    /// The compose file to use.
    Compose(PathBuf),
}

/// Work out what to run for `dir`, tolerating a manifest that no longer parses: a broken
/// `de.toml` must not hide a perfectly good `compose.yaml`.
pub fn compose_plan(dir: &Path) -> ComposePlan {
    if !dir.exists() {
        return ComposePlan::Missing;
    }
    if let Some(file) = Project::from_dir(dir)
        .ok()
        .and_then(|project| project.docker_compose_path().ok().flatten())
    {
        return ComposePlan::Compose(file);
    }
    match find_compose_file_in_dir(dir) {
        Some(file) => {
            let canonical = file.canonicalize().unwrap_or(file);
            ComposePlan::Compose(canonical)
        }
        None => ComposePlan::NoCompose,
    }
}

/// Merge what docker declares with what its containers report, one row per service.
///
/// Exported (and tested) as the pure part of [`health_from`].
pub fn merge_services(declared: Vec<String>, rows: Vec<PsRow>) -> Vec<ServiceStatus> {
    let mut by_service: BTreeMap<String, Vec<PsRow>> = BTreeMap::new();
    for row in rows {
        by_service.entry(row.service.clone()).or_default().push(row);
    }

    let mut services = Vec::new();
    for name in &declared {
        match by_service.remove(name) {
            Some(group) => services.push(combine(name, group)),
            None => services.push(ServiceStatus {
                service: name.clone(),
                state: ContainerState::Absent,
                health: ServiceHealth::NoCheck,
                exit_code: None,
            }),
        }
    }
    // Containers of a service docker no longer declares: they are running, so they count.
    for (name, group) in by_service {
        if !declared.contains(&name) {
            services.push(combine(&name, group));
        }
    }
    services
}

fn combine(service: &str, group: Vec<PsRow>) -> ServiceStatus {
    let state = group
        .iter()
        .map(|r| ContainerState::from_docker(&r.state))
        .find(|s| s.is_running())
        .unwrap_or_else(|| ContainerState::from_docker(&group[0].state));
    let health = group
        .iter()
        .map(|r| ServiceHealth::from_docker(&r.health))
        .max()
        .unwrap_or(ServiceHealth::NoCheck);
    let exit_code = group.iter().find_map(|r| r.exit_code);
    ServiceStatus {
        service: service.to_string(),
        state,
        health,
        exit_code,
    }
}

/// One row of `docker compose ps --format json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsRow {
    pub service: String,
    pub state: String,
    pub health: String,
    pub exit_code: Option<i32>,
}

/// Read `docker compose ps --all --format json`, which docker writes either as one JSON array
/// or as one JSON object per line (both shapes are in the wild).
pub fn parse_ps_rows(text: &str) -> Result<Vec<PsRow>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let values: Vec<serde_json::Value> = if text.starts_with('[') {
        serde_json::from_str(text)
            .map_err(|e| format!("docker compose ps returned something unreadable ({e})"))?
    } else {
        let mut values = Vec::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            values.push(
                serde_json::from_str(line).map_err(|e| {
                    format!("docker compose ps returned something unreadable ({e})")
                })?,
            );
        }
        values
    };

    Ok(values
        .into_iter()
        .filter_map(|value| {
            let object = value.as_object()?;
            let service = string_field(object, &["Service", "service"])?;
            Some(PsRow {
                service,
                state: string_field(object, &["State", "state"]).unwrap_or_default(),
                health: string_field(object, &["Health", "health"]).unwrap_or_default(),
                exit_code: object
                    .get("ExitCode")
                    .or_else(|| object.get("exit_code"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32),
            })
        })
        .collect())
}

fn string_field(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// The service names of `docker compose config --services`, one per line.
pub fn parse_services_list(text: &str) -> Vec<String> {
    let mut services: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    services.sort();
    services.dedup();
    services
}

/// One line of `docker compose ls --format json`: a compose project docker knows is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningComposeProject {
    pub name: String,
    pub status: String,
    pub config_files: Vec<PathBuf>,
}

impl RunningComposeProject {
    /// Whether docker says it is running (its status can mix several projects: `running(2)`).
    pub fn is_running(&self) -> bool {
        self.status
            .split(',')
            .map(str::trim)
            .any(|part| part.starts_with("running"))
    }
}

/// Read `docker compose ls --format json`.
pub fn parse_compose_ls(text: &str) -> Result<Vec<RunningComposeProject>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let values: Vec<serde_json::Value> = serde_json::from_str(text)
        .map_err(|e| format!("docker compose ls returned something unreadable ({e})"))?;

    Ok(values
        .into_iter()
        .filter_map(|value| {
            let object = value.as_object()?;
            let name = object.get("Name")?.as_str()?.to_string();
            let status = object
                .get("Status")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            // ConfigFiles is written as a single string or as an array, depending on version.
            let config_files = match object.get("ConfigFiles") {
                Some(serde_json::Value::Array(items)) => items
                    .iter()
                    .filter_map(|i| i.as_str())
                    .map(PathBuf::from)
                    .collect(),
                Some(serde_json::Value::String(text)) => text
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect(),
                _ => Vec::new(),
            };
            Some(RunningComposeProject {
                name,
                status,
                config_files,
            })
        })
        .collect())
}

/// Run docker, capturing what it said, and turn a failure into a reason.
fn launch(cmd: &mut Command) -> Result<String, DockerError> {
    match cmd.output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(DockerError::NotInstalled),
        Err(e) => Err(DockerError::Other(format!("it could not be run ({e})"))),
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Compose writes some of its complaints to stdout rather than stderr.
            let text = if stderr.trim().is_empty() {
                stdout.as_ref()
            } else {
                stderr.as_ref()
            };
            Err(classify_docker_failure(text))
        }
    }
}

/// `docker compose <args…>` in `dir`, with output captured.
fn run_docker(args: &[&str], dir: &Path) -> Result<String, DockerError> {
    let mut cmd = Command::new("docker");
    cmd.args(args).current_dir(dir);
    launch(&mut cmd)
}

/// Run `docker compose -f <file> <args…>`, capturing what it said.
///
/// This is how the opening and closing sequences start and stop services: the modal shows the
/// friendly reason from [`DockerError`] instead of raw docker output.
pub fn run_compose(file: &Path, dir: &Path, args: &[&str]) -> Result<String, DockerError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose").arg("-f").arg(file).args(args).current_dir(dir);
    launch(&mut cmd)
}

/// Turn the results of the two docker calls into this project's health. Pure, so the merge is
/// tested without docker.
pub fn health_from(
    plan: &ComposePlan,
    declared: Result<String, DockerError>,
    ps: Result<String, DockerError>,
) -> ProjectHealth {
    match plan {
        ComposePlan::Missing => ProjectHealth::MissingFolder,
        ComposePlan::NoCompose => ProjectHealth::NoCompose,
        ComposePlan::Compose(_) => match (declared, ps) {
            (Err(e), _) | (_, Err(e)) => ProjectHealth::Unavailable(e),
            (Ok(declared), Ok(ps)) => match parse_ps_rows(&ps) {
                Ok(rows) => ProjectHealth::Services(merge_services(
                    parse_services_list(&declared),
                    rows,
                )),
                Err(why) => ProjectHealth::Unavailable(DockerError::Other(why)),
            },
        },
    }
}

/// Check one project's services.
pub fn check_project(project: &Slug, dir: &Path) -> ProjectStatus {
    let plan = compose_plan(dir);
    let health = match &plan {
        ComposePlan::Missing => ProjectHealth::MissingFolder,
        ComposePlan::NoCompose => ProjectHealth::NoCompose,
        ComposePlan::Compose(file) => {
            let file = file.to_string_lossy().into_owned();
            let dir = dir.to_path_buf();
            let declared = run_docker(
                &["compose", "-f", &file, "config", "--services"],
                &dir,
            );
            let ps = run_docker(
                &["compose", "-f", &file, "ps", "--all", "--format", "json"],
                &dir,
            );
            health_from(&plan, declared, ps)
        }
    };
    ProjectStatus {
        project: project.clone(),
        dir: dir.to_path_buf(),
        health,
    }
}

/// Check every project of a workspace, in the order they start.
pub fn check_workspace(workspace: &Workspace, now: i64) -> WorkspaceHealth {
    let projects = startup_order(workspace)
        .into_iter()
        .filter_map(|id| {
            let dir = workspace.config().projects.get(&id)?.dir.clone();
            Some(check_project(&id, &dir))
        })
        .collect();
    WorkspaceHealth {
        workspace: workspace.config().name.clone(),
        projects,
        checked_at: now,
    }
}

/// The compose file of every project of `workspace` that has one and whose folder exists.
pub fn compose_files(workspace: &Workspace) -> Vec<PathBuf> {
    workspace
        .config()
        .projects
        .values()
        .filter_map(|project| match compose_plan(&project.dir) {
            ComposePlan::Compose(file) => Some(file),
            _ => None,
        })
        .collect()
}

/// One `docker compose ls` for every workspace: which of them have at least one service
/// running. `Err` when docker cannot be asked, and then no dot is shown anywhere.
pub fn running_workspaces(workspaces: &[&Workspace]) -> Result<std::collections::HashMap<Slug, bool>, DockerError> {
    let running = parse_compose_ls(&run_docker(&["compose", "ls", "--format", "json"], Path::new("."))?)
        .map_err(DockerError::Other)?;
    Ok(workspaces
        .iter()
        .map(|workspace| {
            let up = compose_files(workspace)
                .iter()
                .any(|file| running.iter().any(|project| project.is_running()
                    && project.config_files.iter().any(|c| same_file(c, file))));
            (workspace.config().name.clone(), up)
        })
        .collect())
}

fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(s: &str) -> Slug {
        s.parse().unwrap()
    }

    const PS_ARRAY: &str = r#"[
        {"Name":"shop-api-1","Service":"api","State":"running","Health":"healthy","ExitCode":0},
        {"Name":"shop-db-1","Service":"db","State":"exited","Health":"","ExitCode":0}
    ]"#;

    const PS_LINES: &str = concat!(
        r#"{"Service":"api","State":"running","Health":"starting"}"#,
        "\n",
        r#"{"Service":"worker","State":"exited","ExitCode":137}"#,
        "\n"
    );

    #[test]
    fn ps_comes_back_as_an_array_or_line_by_line() {
        let from_array = parse_ps_rows(PS_ARRAY).unwrap();
        assert_eq!(from_array.len(), 2);
        assert_eq!(from_array[0].service, "api");
        assert_eq!(from_array[0].health, "healthy");
        assert_eq!(from_array[1].state, "exited");

        let from_lines = parse_ps_rows(PS_LINES).unwrap();
        assert_eq!(from_lines.len(), 2);
        assert_eq!(from_lines[0].health, "starting");
        assert_eq!(from_lines[1].exit_code, Some(137));
    }

    #[test]
    fn no_containers_is_an_empty_list_and_garbage_is_said_to_be_garbage() {
        assert!(parse_ps_rows("  \n ").unwrap().is_empty());
        assert!(parse_ps_rows("[]").unwrap().is_empty());

        let err = parse_ps_rows("NAME IMAGE COMMAND CREATED STATUS PORTS\napi nginx").unwrap_err();
        assert!(err.contains("unreadable"), "{err}");

        // A row without a service name is not ours; it is dropped rather than failing the read.
        assert!(parse_ps_rows("[{\"State\":\"running\"}]").unwrap().is_empty());
    }

    #[test]
    fn declared_services_are_named_sorted_and_free_of_blank_lines() {
        assert_eq!(
            parse_services_list("web\n\napi\nweb\n  db  \n"),
            ["api", "db", "web"]
        );
        assert!(parse_services_list("").is_empty());
    }

    #[test]
    fn compose_ls_reads_both_shapes_of_config_files() {
        let one = parse_compose_ls(
            r#"[{"Name":"api","Status":"running(2)","ConfigFiles":"/w/api/compose.yaml"}]"#,
        )
        .unwrap();
        assert_eq!(one[0].config_files, [PathBuf::from("/w/api/compose.yaml")]);
        assert!(one[0].is_running());

        let two = parse_compose_ls(
            r#"[{"Name":"web","Status":"exited(1)","ConfigFiles":["/w/web.yaml","/w/extra.yaml"]}]"#,
        )
        .unwrap();
        assert_eq!(two[0].config_files.len(), 2);
        assert!(!two[0].is_running(), "an exited project is not running");

        // Several services in one project, and a status mixing both.
        let mixed = parse_compose_ls(r#"[{"Name":"x","Status":"running(2), exited(1)","ConfigFiles":""}]"#).unwrap();
        assert!(mixed[0].is_running());
        assert!(parse_compose_ls("not json").is_err());
    }

    #[test]
    fn a_dead_daemon_says_to_start_docker_and_a_missing_docker_says_to_install_it() {
        assert_eq!(
            classify_docker_failure(
                "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?"
            ),
            DockerError::DaemonDown
        );
        assert_eq!(
            classify_docker_failure("error during connect: dial unix /var/run/docker.sock: no such file"),
            DockerError::DaemonDown
        );
        assert_eq!(
            classify_docker_failure("docker: 'compose' is not a docker command."),
            DockerError::ComposeMissing
        );
        assert_eq!(
            classify_docker_failure("service 'api' failed: port 8080 is already allocated\nmore lines"),
            DockerError::Other("service 'api' failed: port 8080 is already allocated".into())
        );

        let down = DockerError::DaemonDown.to_string();
        assert!(down.contains("Start Docker Desktop"), "{down}");
        let missing = DockerError::NotInstalled.to_string();
        assert!(missing.contains("Install Docker Desktop"), "{missing}");
        let compose = DockerError::ComposeMissing.to_string();
        assert!(compose.contains("Compose plugin"), "{compose}");
    }

    #[test]
    fn every_service_running_and_healthy_is_no_exception_at_all() {
        let health = ProjectHealth::Services(merge_services(
            vec!["api".into(), "db".into()],
            parse_ps_rows(PS_ARRAY).unwrap(),
        ));
        // The db in the fixture is exited, so make both running for this case.
        let health_up = ProjectHealth::Services(merge_services(
            vec!["api".into(), "db".into()],
            vec![
                PsRow {
                    service: "api".into(),
                    state: "running".into(),
                    health: "healthy".into(),
                    exit_code: None,
                },
                PsRow {
                    service: "db".into(),
                    state: "running".into(),
                    health: "".into(),
                    exit_code: None,
                },
            ],
        ));
        assert_eq!(health_up.declared(), 2);
        assert_eq!(health_up.running(), 2);

        let ws = WorkspaceHealth {
            workspace: slug("shop"),
            projects: vec![ProjectStatus {
                project: slug("api"),
                dir: PathBuf::from("/w/api"),
                health: health_up,
            }],
            checked_at: 1,
        };
        assert_eq!(ws.overall(), Overall::Up);
        assert_eq!(ws.exception(), None, "healthy is silence");
        assert!(ws.any_running());

        // The same workspace with the db stopped is partial, and says so.
        let partial = WorkspaceHealth {
            projects: vec![ProjectStatus {
                project: slug("api"),
                dir: PathBuf::from("/w/api"),
                health,
            }],
            ..ws.clone()
        };
        assert_eq!(partial.overall(), Overall::Partial);
        assert_eq!(partial.exception().unwrap(), "1 of 2 services are running.");
        assert_eq!(partial.running(), 1);
    }

    #[test]
    fn declared_services_that_never_started_count_as_stopped() {
        let health = ProjectHealth::Services(merge_services(
            vec!["api".into(), "db".into()],
            Vec::new(),
        ));
        assert_eq!(health.declared(), 2);
        assert_eq!(health.running(), 0);
        let ws = WorkspaceHealth {
            workspace: slug("shop"),
            projects: vec![ProjectStatus {
                project: slug("api"),
                dir: PathBuf::from("/w/api"),
                health,
            }],
            checked_at: 1,
        };
        assert_eq!(ws.overall(), Overall::Down);
        assert_eq!(ws.exception().unwrap(), "Services are stopped.");
        assert!(!ws.any_running());
    }

    #[test]
    fn an_unhealthy_service_is_named_and_loud() {
        let health = ProjectHealth::Services(merge_services(
            vec!["api".into(), "worker".into()],
            vec![
                PsRow {
                    service: "api".into(),
                    state: "running".into(),
                    health: "unhealthy".into(),
                    exit_code: None,
                },
                PsRow {
                    service: "worker".into(),
                    state: "running".into(),
                    health: "unhealthy".into(),
                    exit_code: None,
                },
            ],
        ));
        let ws = WorkspaceHealth {
            workspace: slug("shop"),
            projects: vec![ProjectStatus {
                project: slug("shop"),
                dir: PathBuf::from("/w/shop"),
                health,
            }],
            checked_at: 1,
        };
        assert_eq!(ws.unhealthy(), 2);
        assert_eq!(ws.unhealthy_services(), ["shop/api", "shop/worker"]);
        let line = ws.exception().unwrap();
        assert!(line.contains("shop/api, shop/worker are unhealthy"), "{line}");
        assert!(ws.describe().contains("unhealthy"), "{}", ws.describe());
    }

    #[test]
    fn docker_being_unavailable_beats_everything_and_explains_itself() {
        let ws = WorkspaceHealth {
            workspace: slug("shop"),
            projects: vec![
                ProjectStatus {
                    project: slug("api"),
                    dir: PathBuf::from("/w/api"),
                    health: ProjectHealth::Unavailable(DockerError::DaemonDown),
                },
                ProjectStatus {
                    project: slug("docs"),
                    dir: PathBuf::from("/w/docs"),
                    health: ProjectHealth::NoCompose,
                },
            ],
            checked_at: 1,
        };
        assert_eq!(ws.overall(), Overall::Unavailable);
        let line = ws.exception().unwrap();
        assert!(line.contains("Start Docker Desktop"), "{line}");
        assert_eq!(ws.describe(), line);
    }

    #[test]
    fn a_workspace_with_no_services_anywhere_is_not_an_exception() {
        let ws = WorkspaceHealth {
            workspace: slug("docs-only"),
            projects: vec![
                ProjectStatus {
                    project: slug("docs"),
                    dir: PathBuf::from("/w/docs"),
                    health: ProjectHealth::NoCompose,
                },
                ProjectStatus {
                    project: slug("gone"),
                    dir: PathBuf::from("/w/gone"),
                    health: ProjectHealth::MissingFolder,
                },
            ],
            checked_at: 1,
        };
        assert_eq!(ws.overall(), Overall::Empty);
        assert_eq!(ws.exception(), None);
        assert_eq!(ws.describe(), "No services in this workspace.");
    }

    #[test]
    fn health_from_turns_the_two_docker_results_into_one_story() {
        let plan = ComposePlan::Compose(PathBuf::from("/w/api/compose.yaml"));

        let up = health_from(&plan, Ok("api\ndb\n".into()), Ok(PS_ARRAY.into()));
        assert_eq!(up.declared(), 2);
        assert_eq!(up.running(), 1);

        let down = health_from(&plan, Ok("api\n".into()), Err(DockerError::DaemonDown));
        assert_eq!(down.unavailable(), Some(&DockerError::DaemonDown));

        let unreadable = health_from(
            &plan,
            Ok("api\n".into()),
            Err(DockerError::Other("boom".into())),
        );
        assert!(unavailable_reason_is(&unreadable, "Docker said: boom"));

        assert_eq!(
            health_from(&ComposePlan::Missing, Err(DockerError::NotInstalled), Ok(String::new())),
            ProjectHealth::MissingFolder
        );
        assert_eq!(
            health_from(&ComposePlan::NoCompose, Ok(String::new()), Ok(String::new())),
            ProjectHealth::NoCompose
        );
    }

    fn unavailable_reason_is(health: &ProjectHealth, want: &str) -> bool {
        health.unavailable().map(ToString::to_string).as_deref() == Some(want)
    }

    #[test]
    fn the_compose_plan_survives_a_broken_manifest() {
        let root = tempfile::tempdir().unwrap();

        assert_eq!(compose_plan(&root.path().join("nope")), ComposePlan::Missing);

        let plain = root.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(compose_plan(&plain), ComposePlan::NoCompose);

        let with_compose = root.path().join("with-compose");
        std::fs::create_dir_all(&with_compose).unwrap();
        std::fs::write(with_compose.join("compose.yaml"), "services: {}\n").unwrap();
        std::fs::write(with_compose.join("de.toml"), "not toml [").unwrap();
        match compose_plan(&with_compose) {
            ComposePlan::Compose(file) => {
                assert_eq!(file, with_compose.join("compose.yaml").canonicalize().unwrap());
            }
            other => panic!("expected a compose file, got {other:?}"),
        }

        let declared = root.path().join("declared");
        std::fs::create_dir_all(&declared).unwrap();
        std::fs::write(declared.join("custom.yml"), "services: {}\n").unwrap();
        std::fs::write(
            declared.join("de.toml"),
            "[project]\nname = \"api\"\ndocker_compose = \"custom.yml\"\n",
        )
        .unwrap();
        match compose_plan(&declared) {
            ComposePlan::Compose(file) => assert_eq!(
                file,
                declared.join("custom.yml").canonicalize().unwrap()
            ),
            other => panic!("expected the declared file, got {other:?}"),
        }
    }

    #[test]
    fn a_service_row_says_only_what_is_out_of_the_ordinary() {
        let running = ServiceStatus {
            service: "api".into(),
            state: ContainerState::Running,
            health: ServiceHealth::Healthy,
            exit_code: None,
        };
        assert_eq!(running.exception(), None);

        let stopped = ServiceStatus {
            state: ContainerState::Exited,
            exit_code: Some(1),
            ..running.clone()
        };
        assert_eq!(stopped.exception().unwrap(), "stopped");

        let never = ServiceStatus {
            state: ContainerState::Absent,
            health: ServiceHealth::NoCheck,
            ..running
        };
        assert_eq!(never.exception().unwrap(), "never started");
    }
}
