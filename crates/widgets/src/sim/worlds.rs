//! Saved workspaces and their isolation.
//!
//! A workspace has its own tickets, repos, review answers, locks and audit log. Only the open one is "live" in
//! the [`Sim`]; the others are parked as a [`World`] and swapped in when selected, so nothing of one can show in
//! another. With none open the live world is empty and the screens that need a workspace have nothing to show.

use std::collections::BTreeMap;

use super::Sim;
use super::model::*;
use crate::store::Outcome;
use crate::vm::{
    Command, Intent, ProviderVm, RepoName, ToastKind, WelcomeVm, WorkspaceItemVm, WorkspaceName,
    WorkspacesVm,
};

/// Everything that belongs to one workspace.
#[derive(Default)]
pub(crate) struct World {
    pub locks: BTreeMap<RepoName, Lock>,
    pub ws: Workspace,
    pub tickets: Vec<Ticket>,
    pub audit: Vec<AuditEntry>,
    pub responses: Responses,
    pub repos: Vec<RepoCfg>,
}

/// A saved workspace: where it is, when it was last opened, and its world while it is not the open one.
pub(crate) struct Saved {
    pub name: WorkspaceName,
    /// Simulation milliseconds (before the start are in the past); `None` for one that was never opened.
    pub last_used: Option<i64>,
    pub world: Option<World>,
}

/// The command that creates a workspace.
pub const INIT_COMMAND: &str = "de init";

fn cfg(name: &str, base: &str, services: u32) -> RepoCfg {
    RepoCfg {
        name: name.into(),
        base: base.into(),
        prod: "main".into(),
        host: "",
        services,
        overlay_consumer: false,
        consumes: None,
        checks: &[],
    }
}

fn ticket(key: &str, title: &str, kind: &'static str, p: Priority, who: &'static str) -> Ticket {
    let mut t = Ticket::blank(key, title);
    t.kind = kind;
    t.priority = p;
    t.assignee = who;
    t.reporter = "Jane Doe";
    t.sprint = "";
    t.fix_version = "";
    t.estimate = "";
    t
}

/// The data of a workspace other than the main demo one: a handful of tickets and repos, invented.
fn seed_world(name: &str) -> World {
    let (tickets, repos) = match name {
        "threadplay" => (
            vec![
                ticket("TP-12", "Reconnect the lobby after a dropped socket", "Bug", Priority::Highest, "Priya Nair"),
                ticket("TP-15", "Show the round timer on small screens", "Task", Priority::Medium, "Marta Lind"),
                ticket("TP-18", "Leaderboard shows yesterday's scores", "Bug", Priority::High, "Priya Nair"),
            ],
            vec![cfg("tp-server", "develop", 2), cfg("tp-client", "develop", 1)],
        ),
        "hbt" => (
            vec![
                ticket("HBT-204", "Invoice PDF drops the second page", "Bug", Priority::High, "Sam Okoye"),
                ticket("HBT-207", "Export the supplier list as CSV", "Task", Priority::Low, "Sam Okoye"),
            ],
            vec![cfg("hbt-api", "develop", 2), cfg("hbt-web", "develop", 1), cfg("hbt-docs", "main", 0)],
        ),
        // A workspace with nothing in its Review column yet.
        _ => (Vec::new(), vec![cfg(&format!("{name}-app"), "main", 1)]),
    };
    World {
        ws: Workspace {
            branches: repos.iter().map(|r: &RepoCfg| (r.name.clone(), r.base.clone())).collect(),
            dirty: Default::default(),
            up: false,
        },
        tickets,
        repos,
        ..World::default()
    }
}

/// The saved workspaces of a fresh simulation: `shop` is the main demo one and is open; the others were used
/// at various times before.
pub(crate) fn seed_saved() -> Vec<Saved> {
    let hour = 60 * MS_PER_MIN;
    let entry = |name: &str, last: Option<i64>, world: Option<World>| Saved {
        name: name.into(),
        last_used: last,
        world,
    };
    vec![
        entry("shop", Some(0), None),
        entry("threadplay", Some(-3 * hour), Some(seed_world("threadplay"))),
        entry("hbt", Some(-2 * 24 * hour), Some(seed_world("hbt"))),
        entry("quelle", Some(-9 * 24 * hour), Some(seed_world("quelle"))),
        entry("orbit", Some(-20 * 24 * hour), Some(seed_world("orbit"))),
        entry("pair", None, Some(seed_world("pair"))),
    ]
}

impl Sim {
    /// A simulation with no workspace open: the first thing a person sees before picking one.
    pub fn without_workspace() -> Self {
        let mut sim = Self::new();
        sim.apply_close();
        sim
    }

    /// The name of the open workspace.
    pub fn current_workspace(&self) -> Option<&WorkspaceName> {
        self.open_ws.map(|i| &self.saved[i].name)
    }

    fn take_world(&mut self) -> World {
        World {
            locks: std::mem::take(&mut self.locks),
            ws: std::mem::take(&mut self.ws),
            tickets: std::mem::take(&mut self.tickets),
            audit: std::mem::take(&mut self.audit),
            responses: std::mem::take(&mut self.responses),
            repos: std::mem::take(&mut self.repos),
        }
    }

    fn put_world(&mut self, w: World) {
        self.locks = w.locks;
        self.ws = w.ws;
        self.tickets = w.tickets;
        self.audit = w.audit;
        self.responses = w.responses;
        self.repos = w.repos;
    }

    /// Park the open workspace's world in its slot.
    fn park_open(&mut self) {
        if let Some(i) = self.open_ws.take() {
            let w = self.take_world();
            self.saved[i].world = Some(w);
            // Leaving counts as the last use.
            self.saved[i].last_used = Some(self.ms);
        }
    }

    /// Make `target` the open workspace right now, parking the one that was open. The steps that lead up to it are
    /// the sequence's (see `sequence`); this is the change itself.
    pub(crate) fn apply_select(&mut self, target: usize) -> Outcome {
        let name = self.saved[target].name.clone();
        if self.open_ws == Some(target) {
            return Outcome::ok();
        }
        self.park_open();
        let world = self.saved[target].world.take().unwrap_or_default();
        self.put_world(world);
        self.saved[target].last_used = Some(self.ms);
        self.open_ws = Some(target);
        self.ok("workspace.open", None, None, &name);
        Outcome::ok().with_toast(format!("Opened {name}"), ToastKind::Ok, None)
    }

    /// Close the open workspace right now (see `apply_select`).
    pub(crate) fn apply_close(&mut self) -> Outcome {
        let Some(name) = self.current_workspace().cloned() else {
            return Outcome::ok();
        };
        self.park_open();
        self.put_world(World::default());
        Outcome::ok().with_toast(format!("Closed {name}"), ToastKind::Info, None)
    }

    fn item(&self, i: usize) -> WorkspaceItemVm {
        let s = &self.saved[i];
        let open = self.open_ws == Some(i);
        let (repos, up) = if open {
            (self.repos.len(), self.ws.up)
        } else {
            s.world
                .as_ref()
                .map_or((0, false), |w| (w.repos.len(), w.ws.up))
        };
        let projects = if open {
            self.repos.iter().map(|r| r.name.to_string()).collect()
        } else {
            s.world
                .as_ref()
                .map(|w| w.repos.iter().map(|r| r.name.to_string()).collect())
                .unwrap_or_default()
        };
        WorkspaceItemVm {
            name: s.name.clone(),
            projects,
            repos: repos as u32,
            up,
            current: open,
            last_used: match (open, s.last_used) {
                (true, _) => "open".to_string(),
                (false, None) => "never".to_string(),
                (false, Some(at)) => super::ago(((self.ms - at) / MS_PER_MIN).max(0) * 60),
            },
        }
    }

    /// The saved workspaces, most recently used first (the open one counts as used now).
    pub fn workspaces_vm(&self) -> WorkspacesVm {
        let mut order: Vec<usize> = (0..self.saved.len()).collect();
        order.sort_by_key(|i| {
            let used = if self.open_ws == Some(*i) {
                i64::MAX
            } else {
                self.saved[*i].last_used.unwrap_or(i64::MIN)
            };
            std::cmp::Reverse(used)
        });
        WorkspacesVm {
            current: self.current_workspace().cloned(),
            items: order.into_iter().map(|i| self.item(i)).collect(),
        }
    }

    /// Every saved workspace whose name or projects match `query`, for the modal that lists them all.
    pub fn all_workspaces_vm(&self, query: &str) -> crate::vm::AllWorkspacesVm {
        let all = self.workspaces_vm();
        let q = query.trim().to_lowercase();
        let total = all.items.len();
        crate::vm::AllWorkspacesVm {
            query: query.to_string(),
            items: all
                .items
                .into_iter()
                .filter(|w| {
                    q.is_empty()
                        || w.name.to_lowercase().contains(&q)
                        || w.projects.iter().any(|p| p.to_lowercase().contains(&q))
                })
                .collect(),
            total,
        }
    }

    /// What is shown with no workspace open, with the state of the tools given by the caller.
    pub fn welcome_vm(&self, providers: Vec<ProviderVm>) -> WelcomeVm {
        let w = self.workspaces_vm();
        WelcomeVm {
            providers,
            workspaces: w.items,
            init_command: INIT_COMMAND.to_string(),
            current: w.current,
        }
    }
}

/// The intent that opens a workspace.
pub fn open_intent(name: &WorkspaceName) -> Intent {
    Intent::Do(Command::SelectWorkspace(name.clone()))
}
