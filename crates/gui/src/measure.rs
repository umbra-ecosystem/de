//! The saved workspaces, and the open one as it stands right now: its services, its git state and
//! which ticket holds its repos.
//!
//! Reading them is real work (docker and git take seconds), so it runs on a worker thread and the
//! window keeps the last reading until the next one lands. Nothing here runs on the UI thread.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use de_core::git::{GitRepo, RepoStatus};
use de_core::project::git_enabled_in;
use de_core::store::{Kind, Store as Db, tickets};
use de_core::types::Slug;
use de_core::workspace::health::{self, Overall, ProjectHealth, WorkspaceHealth};
use de_core::workspace::{Workspace, registry, startup_order};
use de_widgets::vm::{
    Badge, Branch, Btn, Command, HealthVm, Intent, RepoName, ServicesVm, Tone, WsRow, WorkspaceVm,
};

/// One reading of the open workspace and the saved list, as the window shows it.
#[derive(Default)]
pub struct Measured {
    /// The open workspace's services; `None` while no workspace is open.
    pub health: Option<WorkspaceHealth>,
    /// Which saved workspaces have at least one service running. Empty when docker could not be
    /// asked, and then no dot is shown anywhere.
    pub running: HashMap<Slug, bool>,
    /// Each project of the open workspace as git reported it, in the order they start.
    pub git: Vec<(Slug, Result<RepoStatus, String>)>,
    /// The ticket holding the open workspace's repos, as its key.
    pub active: Option<String>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Read the open workspace (`scope`) and every saved workspace, wherever this is called from.
///
/// Every docker call is best effort: docker missing, or its daemon down, comes back as a check
/// that could not run — never as an error the window has to handle.
pub fn measure(scope: Option<&Slug>, saved: &[Slug]) -> Measured {
    let saved_workspaces: Vec<Workspace> = saved
        .iter()
        .filter_map(|name| registry::load_workspace(name).ok())
        .collect();
    // With nothing to report there is no reason to start docker at all.
    let running = if saved_workspaces.is_empty() {
        HashMap::new()
    } else {
        health::running_workspaces(&saved_workspaces.iter().collect::<Vec<_>>()).unwrap_or_default()
    };
    let workspace = scope.and_then(|scope| registry::load_workspace(scope).ok());
    let (Some(scope), Some(workspace)) = (scope, workspace) else {
        return Measured {
            running,
            ..Measured::default()
        };
    };
    Measured {
        health: Some(health::check_workspace(&workspace, now())),
        running,
        git: git_state(&workspace),
        active: active_ticket(scope),
    }
}

/// Each project's git state, in the order the projects start.
///
/// A project with `[git] enabled = false` has no git state at all: it gets no entry here, so its
/// row shows no branch and no badge rather than a repository that could not be read.
fn git_state(workspace: &Workspace) -> Vec<(Slug, Result<RepoStatus, String>)> {
    startup_order(workspace)
        .into_iter()
        .filter_map(|id| {
            let dir = workspace.config().projects.get(&id)?.dir.clone();
            if !git_enabled_in(&dir) {
                return None;
            }
            let read = GitRepo::open(&dir)
                .and_then(|repo| repo.status())
                .map_err(|e| format!("{e:#}"));
            Some((id, read))
        })
        .collect()
}

/// The ticket holding the workspace's repos, if it has one.
fn active_ticket(scope: &Slug) -> Option<String> {
    Db::open_scoped_for(Some(scope), Kind::State)
        .ok()
        .and_then(|state| tickets::active(&state).ok().flatten())
        .map(|t| t.key.to_string())
}

/// The one badge the strip shows, from what a check found.
///
/// `None` when everything runs or the workspace declares nothing — silence is the normal state —
/// and `None` while no check has happened yet, because nothing is claimed before it is known.
pub fn health_vm(health: Option<&WorkspaceHealth>) -> Option<HealthVm> {
    let health = health?;
    match health.overall() {
        Overall::Up | Overall::Empty => None,
        Overall::Unavailable => Some(HealthVm::unavailable(
            health
                .exception()
                .unwrap_or_else(|| "Docker could not be asked.".to_string()),
        )),
        Overall::Down => HealthVm::stopped(0, health.declared() as u32),
        Overall::Partial if health.unhealthy() > 0 => {
            Some(HealthVm::unhealthy(health.unhealthy_services()))
        }
        Overall::Partial => HealthVm::stopped(health.running() as u32, health.declared() as u32),
    }
}

/// The workspace screen for `name`, from the projects it registers (in the order they start) and
/// the last reading.
///
/// Every project gets a row whether or not the check has run: before it, the Services cell claims
/// nothing (`—`) rather than a count nobody has verified.
pub fn workspace_vm(
    name: Option<&Slug>,
    projects: &[Slug],
    measured: Option<&Measured>,
) -> WorkspaceVm {
    let up = match (name, measured) {
        (Some(name), Some(measured)) => measured.running.get(name).copied().unwrap_or_default(),
        _ => false,
    };
    WorkspaceVm {
        name: name.map(ToString::to_string).unwrap_or_default(),
        up,
        order: projects
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(" \u{2192} "),
        health: health_vm(measured.and_then(|m| m.health.as_ref())),
        rows: projects.iter().map(|id| row(id, measured)).collect(),
        note: measured.and_then(|m| m.active.as_ref()).map(|key| {
            format!(
                "Active ticket {key} holds the repos above. Stashed local changes are \
                 restored when you park or finish."
            )
        }),
        refresh: Btn::new("Refresh", Intent::Do(Command::RefreshHealth)),
        toggle: if up {
            Btn::new("Stop\u{2026}", Intent::Do(Command::StopWorkspace))
        } else {
            Btn::new("Start", Intent::Do(Command::StartWorkspace)).primary()
        },
    }
}

/// One project's row: what the services are doing, which branch it is on, and only what is out of
/// the ordinary.
fn row(id: &Slug, measured: Option<&Measured>) -> WsRow {
    let checked = measured
        .and_then(|m| m.health.as_ref())
        .and_then(|h| h.projects.iter().find(|p| &p.project == id));
    let services = match checked.map(|p| &p.health) {
        // A folder that is gone and a check docker refused claim nothing.
        None | Some(ProjectHealth::MissingFolder) | Some(ProjectHealth::Unavailable(_)) => {
            ServicesVm::Unknown
        }
        Some(health) => ServicesVm::of(Some(health.running() as u32), health.declared() as u32),
    };
    let read = measured.and_then(|m| m.git.iter().find(|(p, _)| p == id));
    let status = read.and_then(|(_, read)| read.as_ref().ok());
    let missing = checked.is_some_and(|p| p.health.missing_folder());
    let mut badges = Vec::new();
    if missing {
        badges.push(Badge::new("folder missing", Tone::Bad));
    } else if read.is_some_and(|(_, read)| read.is_err()) {
        // The repository is there but could not be read, and there is no other reason for it. A
        // project with `[git] enabled = false` has no entry at all and says nothing.
        badges.push(Badge::new("git unreadable", Tone::Warn));
    }
    if status.is_some_and(|s| !s.is_clean()) {
        badges.push(Badge::new("uncommitted changes", Tone::Warn));
    }
    WsRow {
        repo: RepoName::new(id.as_str()),
        services,
        branch: status.map_or_else(Branch::default, |s| Branch::new(s.head_label())),
        badges,
        break_lock: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;

    use de_core::workspace::health::{DockerError, ProjectStatus};

    use super::*;

    fn slug(text: &str) -> Slug {
        Slug::from_str(text).unwrap()
    }

    fn project(health: ProjectHealth) -> ProjectStatus {
        ProjectStatus {
            project: slug("web"),
            dir: PathBuf::from("/tmp/web"),
            health,
        }
    }

    /// The defaults are silence: nothing is claimed before anything has been checked.
    #[test]
    fn no_check_yet_says_nothing() {
        assert_eq!(health_vm(None), None);
        let vm = workspace_vm(None, &[], None);
        assert_eq!(vm.name, "");
        assert!(!vm.up);
        assert!(vm.rows.is_empty());
        assert_eq!(vm.health, None);
        assert_eq!(vm.toggle.label, "Start");
    }

    /// A workspace that could not be asked shows why, once, in the tone of a blocker.
    #[test]
    fn docker_that_could_not_be_asked_says_what_to_do() {
        let health = WorkspaceHealth {
            workspace: slug("shop"),
            projects: vec![project(ProjectHealth::Unavailable(DockerError::DaemonDown))],
            checked_at: 0,
        };
        let vm = health_vm(Some(&health)).expect("a check that could not run is an exception");
        assert_eq!(vm.badge(), Badge::new("Docker unavailable", Tone::Bad));
        assert_eq!(
            vm.notice().as_deref(),
            Some("Docker is not running. Start Docker Desktop, then try again.")
        );
        // The one project shows `—`, not a count nobody could verify.
        let vm = workspace_vm(
            Some(&slug("shop")),
            &[slug("web")],
            Some(&Measured {
                health: Some(health),
                ..Measured::default()
            }),
        );
        assert_eq!(vm.rows[0].services, ServicesVm::Unknown);
    }

    /// Everything running, and a project with nothing to run, are both silence.
    #[test]
    fn all_running_has_no_badge() {
        let health = WorkspaceHealth {
            workspace: slug("shop"),
            projects: vec![project(ProjectHealth::Services(vec![]))],
            checked_at: 0,
        };
        assert_eq!(health_vm(Some(&health)), None);

        let health = WorkspaceHealth {
            workspace: slug("shop"),
            projects: vec![project(ProjectHealth::NoCompose)],
            checked_at: 0,
        };
        assert_eq!(health_vm(Some(&health)), None);
    }

    /// A project behind on its work says so in its own row; a clean one says nothing.
    #[test]
    fn only_uncommitted_work_gets_a_badge() {
        let mut clean = RepoStatus {
            branch: Some("develop".to_string()),
            detached: false,
            head: Some("abc1234".to_string()),
            modified: 0,
            staged: 0,
            untracked: 0,
            upstream: None,
            ahead: 0,
            behind: 0,
            unpushed: 0,
        };
        let measured = Measured {
            git: vec![(slug("web"), Ok(clean.clone()))],
            ..Measured::default()
        };
        let vm = workspace_vm(Some(&slug("shop")), &[slug("web")], Some(&measured));
        assert_eq!(vm.rows[0].badges, Vec::new());
        assert_eq!(vm.rows[0].branch.as_str(), "develop");

        clean.untracked = 2;
        let measured = Measured {
            git: vec![(slug("web"), Ok(clean))],
            ..Measured::default()
        };
        let vm = workspace_vm(Some(&slug("shop")), &[slug("web")], Some(&measured));
        assert_eq!(
            vm.rows[0].badges,
            vec![Badge::new("uncommitted changes", Tone::Warn)]
        );
    }

    /// A repository that is there but could not be read is an exception, and gets a badge of its
    /// own. A project with git turned off has no entry at all, and says nothing.
    #[test]
    fn a_repository_that_could_not_be_read_is_badged() {
        let measured = Measured {
            git: vec![(slug("infra"), Err("is not a git repository".to_string()))],
            ..Measured::default()
        };
        let vm = workspace_vm(Some(&slug("shop")), &[slug("infra")], Some(&measured));
        assert_eq!(
            vm.rows[0].badges,
            vec![Badge::new("git unreadable", Tone::Warn)]
        );
        assert_eq!(vm.rows[0].branch.as_str(), "");

        let vm = workspace_vm(
            Some(&slug("shop")),
            &[slug("infra")],
            Some(&Measured::default()),
        );
        assert_eq!(vm.rows[0].badges, Vec::new());
    }

    /// The toggle follows what the services are doing, and stopping is never the primary button.
    #[test]
    fn the_toggle_stops_what_is_running() {
        let name = slug("shop");
        let mut running = HashMap::new();
        running.insert(name.clone(), true);
        let vm = workspace_vm(
            Some(&name),
            &[],
            Some(&Measured {
                running,
                ..Measured::default()
            }),
        );
        assert!(vm.up);
        assert_eq!(vm.toggle.label, "Stop\u{2026}");
        assert!(!vm.toggle.primary);

        let vm = workspace_vm(Some(&name), &[], Some(&Measured::default()));
        assert!(!vm.up);
        assert_eq!(vm.toggle.label, "Start");
        assert!(vm.toggle.primary);
    }

    /// A project with `[git] enabled = false` is never read at all; a project without the setting
    /// still is, even when reading it does not work.
    #[test]
    fn a_project_with_git_turned_off_is_not_read() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["api", "infra"] {
            std::fs::create_dir_all(temp.path().join(name)).unwrap();
        }
        std::fs::write(
            temp.path().join("infra").join("de.toml"),
            "[project]\nname = \"infra\"\nworkspace = \"shop\"\n\n[git]\nenabled = false\n",
        )
        .unwrap();
        let file = temp.path().join("shop.toml");
        let mut body = String::from("name = \"shop\"\n\n[projects]\n");
        for name in ["api", "infra"] {
            body.push_str(&format!(
                "{name} = {{ dir = \"{}\" }}\n",
                temp.path().join(name).display()
            ));
        }
        std::fs::write(&file, body).unwrap();
        let ws = Workspace::load_from_path(file).unwrap().unwrap();

        // Only `api` was read: `infra` has no git state, so there is nothing to say about it.
        let git = git_state(&ws);
        assert_eq!(git.len(), 1, "{git:?}");
        assert_eq!(git[0].0.as_str(), "api");
        assert!(git[0].1.is_err(), "a plain folder is still attempted");
    }
}
