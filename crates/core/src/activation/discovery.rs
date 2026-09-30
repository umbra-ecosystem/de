//! Reading the workspace repos: which ticket branches exist and which baselines resolve.

use std::path::{Path, PathBuf};

use eyre::Context;

use super::plan::{Baselines, PlanRepo};
use crate::{
    domain::{AuditOutcome, BaselineChoice, TicketKey},
    git::{GitRepo, match_branches, pick_base},
    project::{Project, config::ProjectManifest},
    store::{
        Store,
        audit::{self, NewAuditEntry},
        links::{self, DiscoveredLink},
    },
    workspace::Workspace,
};

use super::actions;

/// A repo of the workspace, with its `de.toml` settings.
#[derive(Debug, Clone)]
pub struct WorkspaceRepo {
    /// The workspace project name (also how tickets link to it and how overlays name it).
    pub name: String,
    pub dir: PathBuf,
    pub manifest: ProjectManifest,
}

impl WorkspaceRepo {
    /// Loads the project in `dir` (its `de.toml` and `.de/config.toml`).
    pub fn load(name: &str, dir: &Path) -> eyre::Result<Self> {
        let project = Project::from_dir(dir)
            .wrap_err_with(|| format!("Failed to load project '{name}' from {}", dir.display()))?;
        Ok(Self {
            name: name.into(),
            dir: dir.into(),
            manifest: project.manifest().clone(),
        })
    }

    /// Every project of `workspace`, in name order.
    pub fn from_workspace(workspace: &Workspace) -> eyre::Result<Vec<Self>> {
        workspace
            .config()
            .projects
            .iter()
            .map(|(id, project)| Self::load(id.as_str(), &project.dir))
            .collect()
    }

    /// The remote to fetch from: the project's `[git] default_remote`, else `origin`.
    pub fn remote(&self) -> &str {
        self.manifest
            .git
            .as_ref()
            .map_or("origin", |g| g.default_remote.as_str())
    }

    pub fn project(&self) -> Project {
        Project::from_parts(self.dir.clone(), self.manifest.clone())
    }
}

/// Branches of one repo that match the ticket key, best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMatches {
    pub repo: String,
    pub branches: Vec<String>,
}

/// What a discovery run found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    /// Repos with at least one matching branch.
    pub matches: Vec<RepoMatches>,
    /// Repos that could not be read (not a git repository, missing directory, ...).
    pub unreadable: Vec<(String, String)>,
}

/// Matches the ticket key against the branches of every repo. Read-only.
pub fn find_matches(ticket: &TicketKey, repos: &[WorkspaceRepo]) -> Discovery {
    let mut discovery = Discovery::default();

    for repo in repos {
        match read_repo(repo, ticket) {
            Ok(read) => {
                if !read.candidates.is_empty() {
                    discovery.matches.push(RepoMatches {
                        repo: repo.name.clone(),
                        branches: read.candidates,
                    });
                }
            }
            Err(e) => discovery
                .unreadable
                .push((repo.name.clone(), format!("{e:#}"))),
        }
    }

    discovery
}

/// Matches the ticket key against the branches of every repo and records the result as `Auto`
/// links. `Manual` and `Excluded` links are never touched (see [`links::apply_discovery`]).
/// A repo with several matching branches is linked without a branch: the choice is the
/// user's, made with a manual link.
pub fn discover_links(
    store: &Store,
    ticket: &TicketKey,
    repos: &[WorkspaceRepo],
    now: i64,
) -> eyre::Result<Discovery> {
    let discovery = find_matches(ticket, repos);

    let found: Vec<DiscoveredLink> = discovery
        .matches
        .iter()
        .map(|m| DiscoveredLink {
            repo: m.repo.clone(),
            branch: match m.branches.as_slice() {
                [only] => Some(only.clone()),
                _ => None,
            },
        })
        .collect();

    // An unreadable repo says nothing: keep whatever link it had rather than dropping it.
    let unread: Vec<&str> = discovery
        .unreadable
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    let keep_existing: Vec<DiscoveredLink> = links::list(store, ticket)?
        .into_iter()
        .filter(|l| {
            unread.contains(&l.repo.as_str()) && l.origin == crate::domain::RepoLinkOrigin::Auto
        })
        .map(|l| DiscoveredLink {
            repo: l.repo,
            branch: l.branch,
        })
        .collect();
    links::apply_discovery(
        store,
        ticket,
        &found.into_iter().chain(keep_existing).collect::<Vec<_>>(),
    )?;

    audit::append(
        store,
        &NewAuditEntry {
            at: now,
            action: actions::LINKS_DISCOVERED.into(),
            ticket: Some(ticket.clone()),
            repo: None,
            details: serde_json::json!({
                "matches": discovery.matches.iter()
                    .map(|m| serde_json::json!({ "repo": m.repo, "branches": m.branches }))
                    .collect::<Vec<_>>(),
                "unreadable": discovery.unreadable.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            }),
            outcome: AuditOutcome::Success,
        },
    )?;

    Ok(discovery)
}

/// The facts of one repo the planner needs.
pub(super) struct RepoRead {
    pub candidates: Vec<String>,
    pub known_branches: Vec<String>,
    pub baselines: Baselines,
}

/// Reads branches once and derives candidates and baselines from them.
pub(super) fn read_repo(repo: &WorkspaceRepo, ticket: &TicketKey) -> eyre::Result<RepoRead> {
    read_repo_with(repo, ticket, None)
}

pub(super) fn read_repo_with(
    repo: &WorkspaceRepo,
    ticket: &TicketKey,
    workspace_default: Option<&str>,
) -> eyre::Result<RepoRead> {
    let git = GitRepo::open(&repo.dir)?;
    let branches = git.logical_branches()?;

    let pick = |choice: BaselineChoice| {
        pick_base(
            &branches,
            &repo.manifest.branches.candidates(choice, workspace_default),
        )
        .map(|b| b.name)
    };

    Ok(RepoRead {
        baselines: Baselines {
            base: pick(BaselineChoice::Base),
            production: pick(BaselineChoice::Production),
            uat: pick(BaselineChoice::Uat),
        },
        known_branches: branches.iter().map(|b| b.name.clone()).collect(),
        candidates: match_branches(branches, ticket.as_str())
            .into_iter()
            .map(|m| m.branch.name)
            .collect(),
    })
}

/// Reads every repo into planner input. A repo that cannot be read is left out and reported
/// in the returned warnings: it is not touched by the activation.
pub fn gather_plan_repos(
    repos: &[WorkspaceRepo],
    ticket: &TicketKey,
    workspace_default: Option<&str>,
) -> (Vec<PlanRepo>, Vec<String>) {
    let mut plan_repos = Vec::new();
    let mut warnings = Vec::new();

    for repo in repos {
        match read_repo_with(repo, ticket, workspace_default) {
            Ok(read) => plan_repos.push(PlanRepo {
                name: repo.name.clone(),
                dir: repo.dir.clone(),
                candidates: read.candidates,
                known_branches: read.known_branches,
                baselines: read.baselines,
                overlay: repo.manifest.overlay.composer.clone(),
                after: repo.manifest.activate.after.clone(),
            }),
            Err(e) => warnings.push(format!(
                "{} could not be read and is left alone: {e:#}",
                repo.name
            )),
        }
    }

    (plan_repos, warnings)
}
