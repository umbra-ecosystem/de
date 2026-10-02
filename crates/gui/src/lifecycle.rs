//! Opening, closing and switching a workspace as a sequence of real steps.
//!
//! Every step of `docs/design/workspace-lifecycle.md` (read the workspace, git state per project,
//! start services, load the data; or the reverse for closing) runs on a worker thread and reports
//! back, so the window never waits on docker or git. Only when the steps are done — or the person
//! chooses to carry on past a failure — does the store actually change the open workspace.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use de_core::domain::TicketKey;
use de_core::git::GitRepo;
use de_core::project::git_enabled_in;
use de_core::store::{Kind as DbKind, Store as Db, tickets};
use de_core::types::Slug;
use de_core::workspace::health::{ComposePlan, compose_plan, parse_services_list, run_compose};
use de_core::workspace::{Workspace, WorkspaceError, registry, startup_order};
use de_widgets::vm::{
    Btn, Command, Intent, PhaseVm, ProgressState, SequenceState, SequenceVm, StepVm,
};

/// How long the modal shows "Ready" before it closes by itself.
const LINGER_MS: i64 = 700;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// What the sequence is doing to which workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Open { to: Slug },
    Close { from: Slug },
    Switch { from: Slug, to: Slug },
}

impl Kind {
    /// What the modal's title says it is doing.
    fn title(&self) -> String {
        match self {
            Kind::Open { to } => format!("Opening {to}"),
            Kind::Close { from } => format!("Closing {from}"),
            Kind::Switch { to, .. } => format!("Switching to {to}"),
        }
    }

    /// Whether the footer's last button offers to carry on closing rather than opening.
    fn closing(&self) -> bool {
        matches!(self, Kind::Close { .. })
    }
}

/// One project's services, with the compose file that runs them.
#[derive(Clone, Debug)]
pub struct ServiceStep {
    pub project: Slug,
    pub file: PathBuf,
    pub dir: PathBuf,
}

/// What one step actually does. Every variant runs off the UI thread.
#[derive(Clone, Debug)]
pub enum Job {
    /// Read the workspace's configuration and check that every project folder is still there.
    Read { scope: Slug },
    /// Read one project's git state.
    Git { project: Slug, dir: PathBuf },
    /// Start one project's services.
    Up {
        project: Slug,
        file: PathBuf,
        dir: PathBuf,
    },
    /// Open the workspace's own data and read what is cached.
    Load { scope: Slug },
    /// The check that gates a close: no ticket may be holding the workspace's repos.
    CheckActive { scope: Slug },
    /// Stop one project's services.
    Down {
        project: Slug,
        file: PathBuf,
        dir: PathBuf,
    },
    /// Record when the workspace was last used.
    Save { scope: Slug },
}

impl Job {
    /// Run the step's work. `Ok` is what the row shows, `Err` a friendly reason it went wrong.
    pub fn run(self) -> Result<String, String> {
        match self {
            Job::Read { scope } => registry::load_workspace(&scope)
                .map_err(|e| e.to_string())
                .and_then(|workspace| read(&workspace)),
            Job::Git { project, dir } => git_state(&project, &dir),
            Job::Up { project, file, dir } => start_services(&project, &file, &dir),
            Job::Load { scope } => load_data(&scope),
            Job::CheckActive { scope } => Db::open_scoped_for(Some(&scope), DbKind::State)
                .map_err(data_unreadable)
                .and_then(|state| tickets::active(&state).map_err(data_unreadable))
                .and_then(|found| report_active(Ok(found.map(|t| t.key)))),
            Job::Down { project, file, dir } => stop_services(&project, &file, &dir),
            Job::Save { scope } => save(&scope),
        }
    }
}

/// The workspace's data could not be read: say which of the two things a person can fix.
fn data_unreadable(e: eyre::Report) -> String {
    format!(
        "The workspace's data could not be read: {e:#}. Check that the data folder is writable."
    )
}

/// Record when the workspace was last used, so the lists keep their order. A workspace whose file
/// is gone has nothing left to record, and that is not a reason to hold up the close.
fn save(scope: &Slug) -> Result<String, String> {
    if matches!(
        registry::load_workspace(scope),
        Err(WorkspaceError::NotFound { .. })
    ) {
        return Ok("nothing to record".to_string());
    }
    registry::stamp_last_used(scope, now())
        .map_err(|e| format!("{e:#}"))
        .map(|()| "kept as you left it".to_string())
}

/// What the closing check says about what it found.
fn report_active(found: Result<Option<TicketKey>, String>) -> Result<String, String> {
    match found {
        Ok(Some(key)) => Err(format!("{key} is active. Park it first.")),
        Ok(None) => Ok("nothing is".to_string()),
        Err(why) => Err(why),
    }
}

/// Read the configuration and check that every project folder is still there.
fn read(workspace: &Workspace) -> Result<String, String> {
    for id in startup_order(workspace) {
        let Some(dir) = workspace.config().projects.get(&id).map(|p| &p.dir) else {
            continue;
        };
        if !dir.exists() {
            return Err(missing_folder(&id, dir, &workspace.config_path));
        }
    }
    Ok(plural(workspace.config().projects.len(), "project"))
}

/// A folder that is not there any more, and where to change it back.
fn missing_folder(id: &Slug, dir: &Path, config: &Path) -> String {
    format!(
        "The folder for {id} is missing: {}. Create it again, or change the project's path in {}.",
        dir.display(),
        config.display()
    )
}

/// One project's git state as the row shows it: `develop, clean`.
fn git_state(project: &Slug, dir: &Path) -> Result<String, String> {
    let status = GitRepo::open(dir)
        .and_then(|repo| repo.status())
        .map_err(|e| format!("Could not read the git state of {project}: {e:#}"))?;
    let changes = status.modified + status.staged + status.untracked;
    let what = if changes == 0 {
        "clean".to_string()
    } else {
        plural(changes, "uncommitted change")
    };
    Ok(format!("{}, {what}", status.head_label()))
}

/// Start one project's services and say how many it brings up.
fn start_services(project: &Slug, file: &Path, dir: &Path) -> Result<String, String> {
    let declared = run_compose(file, dir, &["config", "--services"])
        .map_err(|e| why(project, "start", e))?;
    let count = parse_services_list(&declared).len();
    run_compose(file, dir, &["up", "-d"]).map_err(|e| why(project, "start", e))?;
    Ok(plural(count, "service"))
}

/// Stop one project's services.
fn stop_services(project: &Slug, file: &Path, dir: &Path) -> Result<String, String> {
    run_compose(file, dir, &["down"])
        .map_err(|e| why(project, "stop", e))
        .map(|_| "stopped".to_string())
}

/// The reason a step failed, in the words a person reads: the project first, docker's advice after.
fn why(project: &Slug, verb: &str, e: impl std::fmt::Display) -> String {
    format!("Could not {verb} {project}: {e}")
}

/// A count with its noun: `1 project`, `3 projects`.
fn plural(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

/// Open the workspace's own data and say how much of it there is.
fn load_data(scope: &Slug) -> Result<String, String> {
    let found = crate::sync::load_cached(Some(scope)).map_err(|e| {
        format!(
            "The workspace's data could not be read: {e:#}. Check that the data folder is writable."
        )
    })?;
    Ok(plural(found.len(), "ticket"))
}

/// What starting or stopping the whole workspace's services did.
pub struct SpinDone {
    /// Whether the services were being started or stopped.
    pub up: bool,
    /// The first thing that went wrong, if anything did. Every project is tried either way.
    pub error: Option<String>,
}

/// Start (`up`) or stop every compose project of `ws`, one at a time, in dependency order.
///
/// A workspace whose second project fails still has its first one attempted; the failure shown is
/// the first one, since that is the one to fix.
pub fn spin(ws: &Workspace, up: bool) -> SpinDone {
    let mut error = None;
    for step in service_steps(ws, up) {
        let args: &[&str] = if up { &["up", "-d"] } else { &["down"] };
        if let Err(e) = run_compose(&step.file, &step.dir, args) && error.is_none() {
            error = Some(why(
                &step.project,
                if up { "start" } else { "stop" },
                e,
            ));
        }
    }
    SpinDone { up, error }
}

/// The compose projects of `ws`, in the order they are started (or the reverse when they are
/// stopped, so dependents go first). Projects without a compose file have no step.
pub fn service_steps(ws: &Workspace, up: bool) -> Vec<ServiceStep> {
    let mut order = startup_order(ws);
    if !up {
        order.reverse();
    }
    order
        .into_iter()
        .filter_map(|project| {
            let dir = ws.config().projects.get(&project)?.dir.clone();
            let ComposePlan::Compose(file) = compose_plan(&dir) else {
                return None;
            };
            Some(ServiceStep { project, file, dir })
        })
        .collect()
}

/// One step of a sequence, waiting to run until it is its turn.
struct Step {
    label: String,
    job: Job,
    state: ProgressState,
    /// The result once there is one; empty while it waits.
    detail: String,
}

fn step(label: String, job: Job) -> Step {
    Step {
        label,
        job,
        state: ProgressState::Waiting,
        detail: String::new(),
    }
}

/// One half of a sequence: a switch closes the open workspace, then opens the other.
struct Phase {
    title: String,
    steps: Vec<Step>,
}

/// A workspace opening, closing or switching, step by step.
pub struct Seq {
    kind: Kind,
    phases: Vec<Phase>,
    at: (usize, usize),
    state: SequenceState,
    /// The step at `at` is running; its result arrives here.
    rx: Option<Receiver<Result<String, String>>>,
    linger: i64,
    /// A failed check stopped it: there is nothing to carry on past.
    blocked: bool,
}

impl Seq {
    /// The sequence for `kind`, built out of workspaces the caller has already loaded.
    ///
    /// `Err` is the friendly reason it cannot start at all.
    pub fn build(
        kind: Kind,
        current: Option<&Workspace>,
        target: Option<&Workspace>,
    ) -> Result<Seq, String> {
        let phases = match &kind {
            Kind::Open { to } => vec![open_phase(to, target.ok_or_else(|| not_found(to))?)],
            Kind::Close { from } => vec![close_phase(from, current)],
            Kind::Switch { from, to } => vec![
                close_phase(from, current),
                open_phase(to, target.ok_or_else(|| not_found(to))?),
            ],
        };
        Ok(Seq {
            kind,
            phases,
            at: (0, 0),
            state: SequenceState::Running,
            rx: None,
            linger: 0,
            blocked: false,
        })
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    pub fn state(&self) -> SequenceState {
        self.state
    }

    /// Whether the check at the start failed, so there is nothing to carry on past.
    pub fn blocked(&self) -> bool {
        self.blocked
    }

    /// Drive one step: report its worker's result, then start the next one.
    ///
    /// `true` means everything went fine and the "Ready" beat is over — the caller applies what
    /// the sequence was for.
    pub fn tick(&mut self, millis: i64) -> bool {
        match self.state {
            SequenceState::NeedsDecision => false,
            SequenceState::Ready => {
                self.linger -= millis;
                self.linger <= 0
            }
            SequenceState::Running => {
                let got = self.rx.as_ref().map(|rx| rx.try_recv());
                match got {
                    Some(Ok(result)) => {
                        self.rx = None;
                        self.advance_after(result);
                    }
                    Some(Err(TryRecvError::Disconnected)) => {
                        self.rx = None;
                        self.advance_after(Err(
                            "This step stopped before it finished. Try again.".to_string()
                        ));
                    }
                    Some(Err(TryRecvError::Empty)) | None => {}
                }
                if self.state == SequenceState::Running && self.rx.is_none() {
                    self.spawn_next();
                }
                false
            }
        }
    }

    /// What the modal shows.
    pub fn vm(&self) -> SequenceVm {
        let phases = self
            .phases
            .iter()
            .map(|p| PhaseVm {
                title: p.title.clone(),
                steps: p
                    .steps
                    .iter()
                    .map(|s| StepVm {
                        label: s.label.clone(),
                        detail: s.detail.clone(),
                        state: s.state,
                    })
                    .collect(),
            })
            .collect();
        let all: Vec<&Step> = self.phases.iter().flat_map(|p| &p.steps).collect();
        let done = all
            .iter()
            .filter(|s| {
                matches!(
                    s.state,
                    ProgressState::Done | ProgressState::Failed | ProgressState::Skipped
                )
            })
            .count();
        let mut actions = Vec::new();
        if self.state == SequenceState::NeedsDecision {
            actions.push(Btn::new(
                "Back",
                Intent::Do(Command::AbortSequence),
            ));
            actions.push(Btn::new(
                "Try again",
                Intent::Do(Command::RetrySequence),
            ));
            if !self.blocked {
                actions.push(
                    Btn::new(
                        if self.kind.closing() {
                            "Close anyway"
                        } else {
                            "Open anyway"
                        },
                        Intent::Do(Command::ContinueSequence),
                    )
                    .primary(),
                );
            }
        }
        SequenceVm {
            title: self.kind.title(),
            phases,
            state: self.state,
            progress: format!("{done} of {}", all.len()),
            actions,
        }
    }

    /// Run the current step's job on a worker thread and take its result here.
    fn spawn_next(&mut self) {
        let (p, s) = self.at;
        let current = &mut self.phases[p].steps[s];
        if current.state != ProgressState::Waiting {
            return;
        }
        current.state = ProgressState::Running;
        let job = current.job.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(job.run());
        });
        self.rx = Some(rx);
    }

    /// Record one step's result and move on.
    ///
    /// Only a failed check blocks: nothing may be closed past it, so the rest is skipped and the
    /// person is asked. Any other failure is carried to the end, where they decide whether to
    /// open or close anyway.
    fn advance_after(&mut self, result: Result<String, String>) {
        let (p, s) = self.at;
        let current = self.phases[p].steps[s].job.clone();
        let (state, detail) = match result {
            Ok(ok) => (ProgressState::Done, ok),
            Err(why) => (ProgressState::Failed, why),
        };
        let failed_check =
            matches!(current, Job::CheckActive { .. }) && state == ProgressState::Failed;
        self.phases[p].steps[s].state = state;
        self.phases[p].steps[s].detail = detail;

        // A folder that is gone cannot be read or started: those steps are skipped with the
        // reason rather than run and failed a second time.
        if matches!(current, Job::Read { .. }) && state == ProgressState::Failed {
            self.skip_missing_folders();
        }

        if failed_check {
            self.skip_rest("cannot run past the check");
            self.blocked = true;
            self.state = SequenceState::NeedsDecision;
            return;
        }

        match self.first_waiting() {
            Some(at) => self.at = at,
            None => {
                let failed = self
                    .phases
                    .iter()
                    .flat_map(|p| &p.steps)
                    .any(|s| s.state == ProgressState::Failed);
                self.state = if failed {
                    SequenceState::NeedsDecision
                } else {
                    SequenceState::Ready
                };
                self.linger = LINGER_MS;
            }
        }
    }

    /// Mark every step that is still waiting as skipped, with why it was.
    fn skip_rest(&mut self, reason: &str) {
        for phase in &mut self.phases {
            for s in &mut phase.steps {
                if s.state == ProgressState::Waiting {
                    s.state = ProgressState::Skipped;
                    s.detail = reason.to_string();
                }
            }
        }
    }

    /// Mark the steps whose folder is not there any more as skipped.
    fn skip_missing_folders(&mut self) {
        for phase in &mut self.phases {
            for s in &mut phase.steps {
                if s.state != ProgressState::Waiting {
                    continue;
                }
                let missing = match &s.job {
                    Job::Git { dir, .. } | Job::Up { dir, .. } | Job::Down { dir, .. } => {
                        !dir.exists()
                    }
                    _ => false,
                };
                if missing {
                    s.state = ProgressState::Skipped;
                    s.detail = "the folder is missing".to_string();
                }
            }
        }
    }

    /// The first step still waiting to run, if any.
    fn first_waiting(&self) -> Option<(usize, usize)> {
        for (p, phase) in self.phases.iter().enumerate() {
            for (s, step) in phase.steps.iter().enumerate() {
                if step.state == ProgressState::Waiting {
                    return Some((p, s));
                }
            }
        }
        None
    }
}

/// What a workspace that does not exist says, wherever it is asked for.
fn not_found(to: &Slug) -> String {
    format!(
        "There is no workspace called '{to}'. Run `de workspace list` to see the saved ones, \
         or `de init` in a project folder to create it."
    )
}

/// The steps that open `to`: read it, every project's git state, then the services, then the data.
fn open_phase(to: &Slug, ws: &Workspace) -> Phase {
    let order = startup_order(ws);
    let mut steps = vec![step(
        "Read the workspace".to_string(),
        Job::Read { scope: to.clone() },
    )];
    for id in &order {
        let Some(dir) = ws.config().projects.get(id).map(|p| p.dir.clone()) else {
            continue;
        };
        // A project with `[git] enabled = false` has no git state to read, so it has no step
        // either — the same as a project with no compose file having no service step.
        if !git_enabled_in(&dir) {
            continue;
        }
        steps.push(step(
            format!("Git status \u{b7} {id}"),
            Job::Git {
                project: id.clone(),
                dir,
            },
        ));
    }
    // Every repository is read before any docker step, so nothing has been started yet.
    for id in &order {
        let Some(dir) = ws.config().projects.get(id).map(|p| p.dir.clone()) else {
            continue;
        };
        if let ComposePlan::Compose(file) = compose_plan(&dir) {
            steps.push(step(
                format!("Start services \u{b7} {id}"),
                Job::Up {
                    project: id.clone(),
                    file,
                    dir,
                },
            ));
        }
    }
    steps.push(step(
        "Load the workspace".to_string(),
        Job::Load { scope: to.clone() },
    ));
    Phase {
        title: format!("Opening {to}"),
        steps,
    }
}

/// The steps that close `from`: the check first, then the services in reverse, then the record.
fn close_phase(from: &Slug, ws: Option<&Workspace>) -> Phase {
    let mut steps = vec![step(
        "Check nothing is in progress".to_string(),
        Job::CheckActive { scope: from.clone() },
    )];
    if let Some(ws) = ws {
        for s in service_steps(ws, false) {
            steps.push(step(
                format!("Stop services \u{b7} {}", s.project),
                Job::Down {
                    project: s.project,
                    file: s.file,
                    dir: s.dir,
                },
            ));
        }
    }
    steps.push(step(
        "Save the workspace".to_string(),
        Job::Save { scope: from.clone() },
    ));
    Phase {
        title: format!("Closing {from}"),
        steps,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::str::FromStr;

    use de_core::workspace::health::DockerError;

    use super::*;

    /// A workspace file in a temporary folder, with the project folders it names.
    fn workspace(temp: &Path, name: &str, projects: &[&str]) -> Workspace {
        let file = temp.join(format!("{name}.toml"));
        let mut body = format!("name = \"{name}\"\n\n[projects]\n");
        for project in projects {
            let dir = temp.join(project);
            fs::create_dir_all(&dir).unwrap();
            body.push_str(&format!("{project} = {{ dir = \"{}\" }}\n", dir.display()));
        }
        fs::write(&file, body).unwrap();
        Workspace::load_from_path(file).unwrap().unwrap()
    }

    /// The steps of an opening, in the order the design gives them.
    #[test]
    fn it_reads_the_workspace_and_every_repo_before_touching_docker() {
        let temp = tempfile::tempdir().unwrap();
        let ws = workspace(temp.path(), "shop", &["api", "web"]);
        let seq = Seq::build(
            Kind::Open {
                to: Slug::from_str("shop").unwrap(),
            },
            None,
            Some(&ws),
        )
        .unwrap();

        let labels: Vec<&str> = seq.phases[0].steps.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Read the workspace",
                "Git status \u{b7} api",
                "Git status \u{b7} web",
                // Neither folder has a compose file, so there is nothing to start.
                "Load the workspace",
            ]
        );
        assert_eq!(seq.phases[0].title, "Opening shop");
        assert_eq!(seq.vm().title, "Opening shop");
    }

    /// A project with `[git] enabled = false` has no git state to read, so it has no step: it is
    /// never opened, and never reported as a repository that could not be read.
    #[test]
    fn a_project_with_git_turned_off_gets_no_git_step() {
        let temp = tempfile::tempdir().unwrap();
        let ws = workspace(temp.path(), "shop", &["api", "infra"]);
        fs::write(
            temp.path().join("infra").join("de.toml"),
            "[project]\nname = \"infra\"\nworkspace = \"shop\"\n\n[git]\nenabled = false\n",
        )
        .unwrap();
        let seq = Seq::build(
            Kind::Open {
                to: Slug::from_str("shop").unwrap(),
            },
            None,
            Some(&ws),
        )
        .unwrap();

        let labels: Vec<&str> = seq.phases[0].steps.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Read the workspace",
                // `infra` says it has no git, so there is no step for it at all.
                "Git status \u{b7} api",
                "Load the workspace",
            ]
        );
    }

    /// Steps for a folder that is gone cannot make sense: they are skipped, not failed twice.
    #[test]
    fn a_folder_that_is_gone_makes_its_own_steps_skip() {
        let temp = tempfile::tempdir().unwrap();
        let ws = workspace(temp.path(), "shop", &["api", "web"]);
        fs::remove_dir_all(temp.path().join("web")).unwrap();
        let seq = Seq::build(
            Kind::Open {
                to: Slug::from_str("shop").unwrap(),
            },
            None,
            Some(&ws),
        )
        .unwrap();
        let mut seq = seq;

        seq.advance_after(Err(
            "The folder for web is missing: /gone. Create it again.".to_string()
        ));

        // `api` is still there and runs; `web` is skipped with the reason.
        assert_eq!(seq.phases[0].steps[1].state, ProgressState::Waiting);
        assert_eq!(seq.phases[0].steps[2].state, ProgressState::Skipped);
        assert_eq!(seq.phases[0].steps[2].detail, "the folder is missing");
        assert_eq!(seq.state, SequenceState::Running);
        assert_eq!(seq.at, (0, 1));
    }

    /// A check that fails stops the sequence: nothing may be closed past it, and the footer does
    /// not offer to close anyway.
    #[test]
    fn a_failed_check_blocks_everything_after_it() {
        let temp = tempfile::tempdir().unwrap();
        let ws = workspace(temp.path(), "shop", &["web"]);
        let seq = Seq::build(
            Kind::Close {
                from: Slug::from_str("shop").unwrap(),
            },
            Some(&ws),
            None,
        )
        .unwrap();
        let mut seq = seq;

        seq.advance_after(Err("SHOP-12 is active. Park it first.".to_string()));

        assert_eq!(seq.state, SequenceState::NeedsDecision);
        assert!(seq.blocked());
        assert!(
            seq.phases[0]
                .steps
                .iter()
                .skip(1)
                .all(|s| s.state == ProgressState::Skipped),
            "everything after the check is skipped"
        );
        let vm = seq.vm();
        let labels: Vec<&str> = vm.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, vec!["Back", "Try again"]);
        assert_eq!(vm.progress, "2 of 2", "the check and the record that was skipped");
    }

    /// A step that went wrong anywhere else is carried to the end, where the person decides.
    #[test]
    fn a_failed_step_ends_with_a_choice() {
        let temp = tempfile::tempdir().unwrap();
        let ws = workspace(temp.path(), "shop", &["web"]);
        let seq = Seq::build(
            Kind::Open {
                to: Slug::from_str("shop").unwrap(),
            },
            None,
            Some(&ws),
        )
        .unwrap();
        let mut seq = seq;

        seq.advance_after(Ok("1 project".to_string()));
        seq.advance_after(Err("Could not read the git state of web: not a repository.".to_string()));
        seq.advance_after(Ok("Load the workspace".to_string()));

        assert_eq!(seq.state, SequenceState::NeedsDecision);
        assert!(!seq.blocked());
        let vm = seq.vm();
        let labels: Vec<&str> = vm.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, vec!["Back", "Try again", "Open anyway"]);
        // Nothing finished is re-run by the tick: the person chooses.
        assert!(!seq.tick(10_000));
    }

    /// Everything that went fine shows "Ready" and closes itself after a short beat.
    #[test]
    fn a_sequence_that_all_went_fine_finishes_by_itself() {
        let temp = tempfile::tempdir().unwrap();
        let ws = workspace(temp.path(), "shop", &["web"]);
        let seq = Seq::build(
            Kind::Open {
                to: Slug::from_str("shop").unwrap(),
            },
            None,
            Some(&ws),
        )
        .unwrap();
        let mut seq = seq;

        seq.advance_after(Ok("1 project".to_string()));
        seq.advance_after(Ok("develop, clean".to_string()));
        seq.advance_after(Ok("Load the workspace".to_string()));

        assert_eq!(seq.state, SequenceState::Ready);
        assert_eq!(seq.vm().progress, "3 of 3");
        assert!(!seq.tick(LINGER_MS - 1));
        assert!(seq.tick(1));
    }

    /// A switch closes the open workspace first, then opens the other.
    #[test]
    fn a_switch_closes_before_it_opens() {
        let temp = tempfile::tempdir().unwrap();
        let from = workspace(temp.path(), "shop", &["web"]);
        let to = workspace(temp.path(), "yard", &["api"]);
        let seq = Seq::build(
            Kind::Switch {
                from: Slug::from_str("shop").unwrap(),
                to: Slug::from_str("yard").unwrap(),
            },
            Some(&from),
            Some(&to),
        )
        .unwrap();

        let titles: Vec<&str> = seq.phases.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, vec!["Closing shop", "Opening yard"]);
        assert_eq!(seq.vm().title, "Switching to yard");
        // The close has no compose file to stop, so it is check, then record.
        assert_eq!(
            seq.phases[0]
                .steps
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Check nothing is in progress", "Save the workspace"]
        );
    }

    /// A workspace nobody can load still has a sequence that fails on the check or the record.
    #[test]
    fn a_workspace_whose_file_is_gone_still_closes() {
        let seq = Seq::build(
            Kind::Close {
                from: Slug::from_str("gone").unwrap(),
            },
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            seq.phases[0]
                .steps
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Check nothing is in progress", "Save the workspace"]
        );
    }

    /// The services of the projects that have them, started in dependency order and stopped in
    /// the reverse, so dependents go first.
    #[test]
    fn the_services_start_in_order_and_stop_in_reverse() {
        let temp = tempfile::tempdir().unwrap();
        let ws = workspace(temp.path(), "shop", &["api", "web"]);
        for project in ["api", "web"] {
            fs::write(temp.path().join(project).join("compose.yaml"), "services:\n").unwrap();
        }

        let up: Vec<String> = service_steps(&ws, true)
            .into_iter()
            .map(|s| s.project.to_string())
            .collect();
        let down: Vec<String> = service_steps(&ws, false)
            .into_iter()
            .map(|s| s.project.to_string())
            .collect();
        assert_eq!(up, ["api", "web"]);
        assert_eq!(down, ["web", "api"]);
    }

    /// Reading the configuration says how many projects there are, and names the file to fix when
    /// one of their folders is gone.
    #[test]
    fn reading_the_workspace_says_its_projects_and_names_a_folder_that_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let ws = workspace(temp.path(), "shop", &["api", "web"]);
        assert_eq!(read(&ws).unwrap(), "2 projects");

        fs::remove_dir_all(temp.path().join("api")).unwrap();
        let why = read(&ws).unwrap_err();
        assert!(why.starts_with("The folder for api is missing: "), "{why}");
        assert!(
            why.contains(&temp.path().join("shop.toml").display().to_string()),
            "{why}"
        );
    }

    /// The closing check is the one thing that may stop a close, and it says what to do.
    #[test]
    fn an_active_ticket_stops_the_close_with_something_to_do_about_it() {
        let key = TicketKey::from_str("SHOP-142").unwrap();
        assert_eq!(
            report_active(Ok(Some(key))).unwrap_err(),
            "SHOP-142 is active. Park it first."
        );
        assert_eq!(report_active(Ok(None)).unwrap(), "nothing is");
        assert_eq!(
            report_active(Err("the data folder is not writable".to_string())).unwrap_err(),
            "the data folder is not writable"
        );
    }

    /// A git step reads the branch and counts what is not committed yet.
    #[test]
    fn a_git_step_reads_the_branch_and_the_uncommitted_work() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("web");
        fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(status.status.success(), "{args:?}: {status:?}");
        };
        git(&["init", "-b", "develop"]);
        fs::write(dir.join("notes.txt"), "later").unwrap();

        let project = Slug::from_str("web").unwrap();
        assert_eq!(
            git_state(&project, &dir).unwrap(),
            "develop, 1 uncommitted change"
        );
        // A folder that is not a repository says which project could not be read.
        let elsewhere = temp.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        assert!(
            git_state(&project, &elsewhere)
                .unwrap_err()
                .starts_with("Could not read the git state of web: "),
            "a plain friendly reason"
        );
    }

    /// The reason a docker step failed names the project first, then gives docker's own advice.
    #[test]
    fn a_docker_failure_names_the_project_and_says_what_to_do() {
        let project = Slug::from_str("web").unwrap();
        assert_eq!(
            why(&project, "start", DockerError::DaemonDown),
            "Could not start web: Docker is not running. Start Docker Desktop, then try again."
        );
        assert_eq!(plural(1, "service"), "1 service");
        assert_eq!(plural(3, "service"), "3 services");
    }
}
