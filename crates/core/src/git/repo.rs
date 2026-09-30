//! Read-only access to a repository through `git2`: status, branches, refs and history.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use eyre::{Context, eyre};
use git2::{BranchType, ErrorCode, Repository, Sort, StatusOptions};

use super::{keymatch::name_contains_key, runner::GitRunner};

/// Handle to a local git repository.
pub struct GitRepo {
    repo: Repository,
    path: PathBuf,
}

impl std::fmt::Debug for GitRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitRepo").field("path", &self.path).finish()
    }
}

/// Structured working-tree and branch state of one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    /// Current branch name; `None` when HEAD is detached.
    pub branch: Option<String>,
    pub detached: bool,
    /// Full sha of HEAD; `None` for a repository without commits.
    pub head: Option<String>,
    /// Tracked files modified or deleted in the working tree (and conflicts).
    pub modified: usize,
    /// Files with changes staged in the index.
    pub staged: usize,
    /// Untracked (not ignored) files.
    pub untracked: usize,
    /// Upstream as `remote/branch`, when the branch has one that still exists.
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// Commits that exist only locally: ahead of the upstream, or, without an
    /// upstream, not reachable from any remote-tracking branch.
    pub unpushed: usize,
}

impl RepoStatus {
    /// No tracked modifications, staged changes or untracked files.
    pub fn is_clean(&self) -> bool {
        self.modified == 0 && self.staged == 0 && self.untracked == 0
    }

    /// Anything that would be lost or left behind by tearing the checkout down.
    pub fn has_uncommitted_or_unpushed(&self) -> bool {
        !self.is_clean() || self.unpushed > 0
    }

    /// `branch` for a branch, the short sha for a detached HEAD.
    pub fn head_label(&self) -> String {
        match (&self.branch, &self.head) {
            (Some(branch), _) => branch.clone(),
            (None, Some(head)) => format!("detached at {}", short_sha(head)),
            (None, None) => "(no commits)".into(),
        }
    }
}

pub fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Local,
    Remote,
}

/// One branch ref, local or remote-tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    /// Logical name without the remote prefix, e.g. `feature/PROJ-1-x`.
    pub name: String,
    /// The name git resolves: `feature/PROJ-1-x` or `origin/feature/PROJ-1-x`.
    pub refname: String,
    pub kind: BranchKind,
    pub remote: Option<String>,
    pub tip: String,
    /// Commit time of the tip, seconds since the epoch.
    pub tip_time: i64,
    /// For a local branch, the upstream it tracks (`origin/x`).
    pub upstream: Option<String>,
    /// For a remote branch, whether some local branch tracks it.
    pub tracked_by_local: bool,
}

/// A local branch and its remote counterparts folded into one logical branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalBranch {
    pub name: String,
    pub local: Option<BranchInfo>,
    pub remotes: Vec<BranchInfo>,
}

impl LogicalBranch {
    /// Newest tip among all refs of this branch.
    pub fn tip_time(&self) -> i64 {
        self.local
            .iter()
            .chain(self.remotes.iter())
            .map(|b| b.tip_time)
            .max()
            .unwrap_or(0)
    }

    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }
}

/// A branch whose name contains a ticket key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchMatch {
    pub branch: LogicalBranch,
    /// The key matched more than one logical branch: stale or duplicate candidates.
    pub duplicate: bool,
}

/// A base branch found from a preference list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseBranch {
    pub name: String,
    pub local: bool,
    pub remote_refs: Vec<String>,
}

impl BaseBranch {
    /// A ref that resolves: the local branch when there is one, else the first remote one.
    pub fn refname(&self) -> &str {
        if self.local {
            &self.name
        } else {
            self.remote_refs.first().map_or(&self.name, |r| r.as_str())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub sha: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
}

impl GitRepo {
    /// Open the repository whose working tree is exactly `path`.
    pub fn open(path: &Path) -> eyre::Result<Self> {
        let repo = Repository::open(path)
            .wrap_err_with(|| format!("Failed to open git repository at {}", path.display()))?;
        if repo.is_bare() {
            return Err(eyre!("{} is a bare repository", path.display()));
        }
        let workdir = repo
            .workdir()
            .map_or_else(|| path.into(), Path::to_path_buf);
        Ok(Self {
            repo,
            path: workdir,
        })
    }

    /// The working tree directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn runner(&self) -> GitRunner {
        GitRunner::new(&self.path)
    }

    pub(crate) fn inner(&self) -> &Repository {
        &self.repo
    }

    pub fn status(&self) -> eyre::Result<RepoStatus> {
        let head_ref = self
            .repo
            .find_reference("HEAD")
            .wrap_err("Failed to read HEAD")?;
        let branch: Option<String> = head_ref
            .symbolic_target()
            .map(|t| String::from(t.strip_prefix("refs/heads/").unwrap_or(t)));
        let detached = branch.is_none();
        let head_oid = self.repo.head().ok().and_then(|h| h.target());

        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);
        let statuses = self
            .repo
            .statuses(Some(&mut opts))
            .wrap_err("Failed to read working tree status")?;

        let (mut modified, mut staged, mut untracked) = (0, 0, 0);
        for entry in statuses.iter() {
            let s = entry.status();
            if s.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE,
            ) {
                staged += 1;
            }
            if s.intersects(
                git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::WT_TYPECHANGE
                    | git2::Status::CONFLICTED,
            ) {
                modified += 1;
            }
            if s.contains(git2::Status::WT_NEW) {
                untracked += 1;
            }
        }

        let mut upstream = None;
        let (mut ahead, mut behind) = (0, 0);
        if let (Some(name), Some(head_oid)) = (&branch, head_oid)
            && let Ok(local) = self.repo.find_branch(name, BranchType::Local)
            && let Ok(up) = local.upstream()
            && let (Ok(Some(up_name)), Some(up_oid)) = (up.name(), up.get().target())
        {
            upstream = Some(String::from(up_name));
            (ahead, behind) = self.repo.graph_ahead_behind(head_oid, up_oid)?;
        }

        let unpushed = match head_oid {
            None => 0,
            Some(_) if upstream.is_some() => ahead,
            Some(oid) => self.count_not_on_remotes(oid)?,
        };

        Ok(RepoStatus {
            branch,
            detached,
            head: head_oid.map(|o| o.to_string()),
            modified,
            staged,
            untracked,
            upstream,
            ahead,
            behind,
            unpushed,
        })
    }

    /// Commits reachable from `oid` but from no remote-tracking branch.
    fn count_not_on_remotes(&self, oid: git2::Oid) -> eyre::Result<usize> {
        let mut walk = self.repo.revwalk()?;
        walk.push(oid)?;
        for reference in self.repo.references()? {
            let reference = reference?;
            if !reference.is_remote() {
                continue;
            }
            if let Some(target) = reference.resolve().ok().and_then(|r| r.target()) {
                walk.hide(target)?;
            }
        }
        Ok(walk.count())
    }

    /// Every local and remote-tracking branch, one entry per ref.
    pub fn branches(&self) -> eyre::Result<Vec<BranchInfo>> {
        let remotes: Vec<String> = self
            .repo
            .remotes()?
            .iter()
            .flatten()
            .map(String::from)
            .collect();

        let mut infos = Vec::new();
        let mut tracked = BTreeSet::new();

        for item in self.repo.branches(None)? {
            let (branch, kind) = item?;
            let Some(refname) = branch.name()?.map(String::from) else {
                continue;
            };
            let Ok(commit) = branch.get().peel_to_commit() else {
                continue;
            };

            let (name, remote, kind, upstream) = match kind {
                BranchType::Local => {
                    let upstream = branch
                        .upstream()
                        .ok()
                        .and_then(|u| u.name().ok().flatten().map(String::from));
                    if let Some(upstream) = &upstream {
                        tracked.insert(upstream.clone());
                    }
                    (refname.clone(), None, BranchKind::Local, upstream)
                }
                BranchType::Remote => {
                    // Longest match, since remote names may themselves contain '/'.
                    let Some(remote) = remotes
                        .iter()
                        .filter(|r| refname.starts_with(&format!("{r}/")))
                        .max_by_key(|r| r.len())
                    else {
                        continue;
                    };
                    let name = &refname[remote.len() + 1..];
                    if name == "HEAD" {
                        continue;
                    }
                    (
                        String::from(name),
                        Some(remote.clone()),
                        BranchKind::Remote,
                        None,
                    )
                }
            };

            infos.push(BranchInfo {
                name,
                refname,
                kind,
                remote,
                tip: commit.id().to_string(),
                tip_time: commit.time().seconds(),
                upstream,
                tracked_by_local: false,
            });
        }

        for info in &mut infos {
            if info.kind == BranchKind::Remote {
                info.tracked_by_local = tracked.contains(&info.refname);
            }
        }

        infos.sort_by(|a, b| a.refname.cmp(&b.refname));
        Ok(infos)
    }

    /// Branches with a local branch and its remote counterparts folded together.
    pub fn logical_branches(&self) -> eyre::Result<Vec<LogicalBranch>> {
        Ok(group_branches(self.branches()?))
    }

    /// Branches whose name contains `key`; see [`name_contains_key`] for the rules.
    pub fn find_branches_for_key(&self, key: &str) -> eyre::Result<Vec<BranchMatch>> {
        Ok(match_branches(self.logical_branches()?, key))
    }

    /// The first of `candidates` that exists as a local or remote branch.
    pub fn resolve_base(&self, candidates: &[&str]) -> eyre::Result<Option<BaseBranch>> {
        let branches = self.logical_branches()?;
        Ok(pick_base(&branches, candidates))
    }

    /// Like [`resolve_base`](Self::resolve_base), returning just the branch name.
    pub fn default_branch(&self, candidates: &[&str]) -> eyre::Result<Option<String>> {
        Ok(self.resolve_base(candidates)?.map(|b| b.name))
    }

    pub(crate) fn commit(&self, rev: &str) -> eyre::Result<git2::Commit<'_>> {
        self.repo
            .revparse_single(rev)
            .and_then(|o| o.peel_to_commit())
            .wrap_err_with(|| format!("Failed to resolve '{rev}' in {}", self.path.display()))
    }

    /// Full sha that `rev` (branch, remote branch, sha, tag) points at.
    pub fn rev_parse(&self, rev: &str) -> eyre::Result<String> {
        Ok(self.commit(rev)?.id().to_string())
    }

    /// Sha of the merge base of `a` and `b`, or `None` for unrelated histories.
    pub fn merge_base(&self, a: &str, b: &str) -> eyre::Result<Option<String>> {
        let (a, b) = (self.commit(a)?.id(), self.commit(b)?.id());
        match self.repo.merge_base(a, b) {
            Ok(oid) => Ok(Some(oid.to_string())),
            Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Whether `a` is an ancestor of `b` (or the same commit).
    pub fn is_ancestor(&self, a: &str, b: &str) -> eyre::Result<bool> {
        let (a, b) = (self.commit(a)?.id(), self.commit(b)?.id());
        Ok(a == b || self.repo.graph_descendant_of(b, a)?)
    }

    /// Commits reachable from `head` but not from `base` (`base..head`), newest first.
    pub fn commits_not_in(&self, base: &str, head: &str) -> eyre::Result<Vec<CommitInfo>> {
        let (base, head) = (self.commit(base)?.id(), self.commit(head)?.id());
        let mut walk = self.repo.revwalk()?;
        walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
        walk.push(head)?;
        walk.hide(base)?;

        let mut commits = Vec::new();
        for oid in walk {
            let commit = self.repo.find_commit(oid?)?;
            commits.push(CommitInfo {
                sha: commit.id().to_string(),
                summary: String::from(commit.summary().unwrap_or_default()),
                author: String::from(commit.author().name().unwrap_or_default()),
                time: commit.time().seconds(),
            });
        }
        Ok(commits)
    }
}

/// Fold local and remote refs with the same logical name together.
pub fn group_branches(infos: Vec<BranchInfo>) -> Vec<LogicalBranch> {
    let mut grouped: BTreeMap<String, LogicalBranch> = BTreeMap::new();
    for info in infos {
        let entry = grouped
            .entry(info.name.clone())
            .or_insert_with(|| LogicalBranch {
                name: info.name.clone(),
                local: None,
                remotes: Vec::new(),
            });
        match info.kind {
            BranchKind::Local => entry.local = Some(info),
            BranchKind::Remote => entry.remotes.push(info),
        }
    }

    let mut branches: Vec<_> = grouped.into_values().collect();
    for branch in &mut branches {
        branch.remotes.sort_by(|a, b| a.refname.cmp(&b.refname));
    }
    branches
}

/// Keep the branches matching `key`, ordered local first then newest first.
pub fn match_branches(branches: Vec<LogicalBranch>, key: &str) -> Vec<BranchMatch> {
    let mut matched: Vec<LogicalBranch> = branches
        .into_iter()
        .filter(|b| name_contains_key(&b.name, key))
        .collect();

    matched.sort_by(|a, b| {
        b.is_local()
            .cmp(&a.is_local())
            .then_with(|| b.tip_time().cmp(&a.tip_time()))
            .then_with(|| a.name.cmp(&b.name))
    });

    let duplicate = matched.len() > 1;
    matched
        .into_iter()
        .map(|branch| BranchMatch { branch, duplicate })
        .collect()
}

/// First candidate present in `branches`.
pub fn pick_base(branches: &[LogicalBranch], candidates: &[&str]) -> Option<BaseBranch> {
    candidates.iter().find_map(|candidate| {
        branches
            .iter()
            .find(|b| b.name == *candidate)
            .map(|b| BaseBranch {
                name: b.name.clone(),
                local: b.is_local(),
                remote_refs: b.remotes.iter().map(|r| r.refname.clone()).collect(),
            })
    })
}
