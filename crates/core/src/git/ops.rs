//! Mutations and network access, done by running the `git` binary.
//!
//! Nothing here forces anything: no `--force`, no `reset --hard`, no forced checkout.

use std::path::Path;

use eyre::{Context, eyre};
use git2::BranchType;

use super::repo::{BranchKind, GitRepo, short_sha};

/// What `switch` should end up on.
enum SwitchTarget<'a> {
    Branch(&'a str),
    Commit(&'a str),
}

/// A stash entry, identified by the commit git made for it plus the label it
/// was created with. Stash indexes shift as stashes come and go, so they are
/// never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashRef {
    pub label: String,
    pub commit: String,
}

/// One line of `git stash list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    pub index: usize,
    pub commit: String,
    pub message: String,
}

/// What `switch` does with a dirty working tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OnDirty {
    /// Stash (including untracked files) under an automatic label, then switch.
    #[default]
    Stash,
    /// Same as `Stash` with a caller-chosen label.
    StashLabelled(String),
    /// Fail without touching anything.
    Abort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchOutcome {
    /// Branch that was checked out before; `None` if HEAD was detached.
    pub previous_branch: Option<String>,
    /// HEAD commit before the switch (restores a detached HEAD).
    pub previous_commit: Option<String>,
    /// The stash made for a dirty tree; pop it to restore.
    pub stash: Option<StashRef>,
    /// A local tracking branch was created from a remote-only branch.
    pub created_tracking_branch: bool,
    /// The branch was already checked out; nothing was done.
    pub already_on_branch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastForward {
    UpToDate,
    Advanced {
        from: String,
        to: String,
    },
    /// Local and upstream both have commits the other lacks; nothing was changed.
    Diverged {
        ahead: usize,
        behind: usize,
    },
    NoUpstream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged {
        commit: String,
    },
    UpToDate,
    /// The merge conflicted and was aborted; the worktree is clean again.
    Conflict {
        files: Vec<String>,
    },
}

/// Checks a remote or branch name that comes from configuration (`[git] default_remote`,
/// `[branches] uat`) before it is put into a `git` argument list or a refspec. Refuses
/// rather than sanitizes: a value that starts with `-` would be read as an option, and
/// whitespace, `:`, `+`, `..`, `~^?*[\`, control characters, `@{`, a leading or trailing `/`
/// (or `.`), `//` or a `.lock` suffix are dangerous in a refspec or not a valid name at all.
pub fn validate_ref_component(what: &str, value: &str) -> eyre::Result<()> {
    let refuse = |why: &str| Err(eyre!("{what} {value:?} is not usable: {why}"));
    if value.is_empty() {
        return refuse("it is empty");
    }
    if value.starts_with('-') {
        return refuse("it starts with '-' and would be read as an option");
    }
    if value.starts_with('/') || value.ends_with('/') {
        return refuse("it starts or ends with '/'");
    }
    if value.contains("//") {
        return refuse("it contains '//'");
    }
    if value.contains("..") {
        return refuse("it contains '..'");
    }
    if value.contains("@{") || value == "@" {
        return refuse("it contains '@{' or is '@'");
    }
    if value.starts_with('.') || value.ends_with('.') || value.ends_with(".lock") {
        return refuse("it starts or ends with '.' or ends with '.lock'");
    }
    if let Some(c) = value
        .chars()
        .find(|c| c.is_control() || c.is_whitespace() || ":+~^?*[\\".contains(*c))
    {
        return refuse(&format!("it contains {c:?}"));
    }
    Ok(())
}

impl GitRepo {
    /// `git fetch -- <remote>`. Does not prune. The remote is validated first.
    pub fn fetch(&self, remote: &str) -> eyre::Result<()> {
        validate_ref_component("remote", remote)?;
        self.runner()
            .run(&["fetch", "--", remote])
            .wrap_err_with(|| format!("Failed to fetch '{remote}'"))?;
        Ok(())
    }

    /// Advance local `branch` to its upstream if that is a fast-forward.
    ///
    /// Works on the checked-out branch and on any other local branch (without
    /// checking it out). Call [`fetch`](Self::fetch) first to see new commits.
    pub fn fast_forward(&self, branch: &str) -> eyre::Result<FastForward> {
        let local = self
            .inner()
            .find_branch(branch, BranchType::Local)
            .wrap_err_with(|| format!("No local branch '{branch}'"))?;
        let Some(upstream) = local
            .upstream()
            .ok()
            .and_then(|u| u.name().ok().flatten().map(String::from))
        else {
            return Ok(FastForward::NoUpstream);
        };

        let from = self.rev_parse(branch)?;
        let upstream_sha = self.rev_parse(&upstream)?;
        let (ahead, behind) = self.inner().graph_ahead_behind(
            git2::Oid::from_str(&from)?,
            git2::Oid::from_str(&upstream_sha)?,
        )?;

        if behind == 0 {
            return Ok(FastForward::UpToDate);
        }
        if ahead > 0 {
            return Ok(FastForward::Diverged { ahead, behind });
        }

        let runner = self.runner();
        if self.status()?.branch.as_deref() == Some(branch) {
            runner.run(&["merge", "--ff-only", &upstream])?;
        } else {
            // Updates the ref without a checkout; git refuses anything but a fast-forward.
            runner.run(&["fetch", ".", &format!("{upstream}:{branch}")])?;
        }
        Ok(FastForward::Advanced {
            from,
            to: self.rev_parse(branch)?,
        })
    }

    /// Stash tracked changes and untracked files under `label`. Returns `None`
    /// when there was nothing to stash.
    pub fn stash_push(&self, label: &str) -> eyre::Result<Option<StashRef>> {
        if self.status()?.is_clean() {
            return Ok(None);
        }

        let runner = self.runner();
        let before = self.stash_top()?;
        runner
            .run(&["stash", "push", "--include-untracked", "-m", label])
            .wrap_err("Failed to stash changes")?;
        let after = self.stash_top()?;

        // A push that stashed nothing leaves the top entry unchanged.
        match after {
            Some(commit) if Some(&commit) != before.as_ref() => Ok(Some(StashRef {
                label: label.into(),
                commit,
            })),
            _ => Ok(None),
        }
    }

    fn stash_top(&self) -> eyre::Result<Option<String>> {
        let out = self
            .runner()
            .run_raw(&["rev-parse", "--verify", "--quiet", "refs/stash"])?;
        Ok(out.success.then(|| out.stdout.trim().into()))
    }

    /// Stashes, newest first.
    pub fn stash_list(&self) -> eyre::Result<Vec<StashEntry>> {
        let out = self
            .runner()
            .run(&["stash", "list", "--format=%H%x09%gs"])?;
        Ok(out
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let (commit, message) = line.split_once('\t')?;
                Some(StashEntry {
                    index,
                    commit: commit.into(),
                    message: message.into(),
                })
            })
            .collect())
    }

    /// Apply and drop the stash `stash` refers to, wherever it now sits in the
    /// stack. Found by its commit; falls back to the label. On conflicts git
    /// keeps the stash and this returns an error.
    pub fn stash_pop(&self, stash: &StashRef) -> eyre::Result<()> {
        let entries = self.stash_list()?;
        let suffix = format!(": {}", stash.label);
        let entry = entries
            .iter()
            .find(|e| e.commit == stash.commit)
            .or_else(|| entries.iter().find(|e| e.message.ends_with(&suffix)))
            .ok_or_else(|| eyre!("Stash '{}' was not found (already popped?)", stash.label))?;

        let target = format!("stash@{{{}}}", entry.index);
        let runner = self.runner();
        let first = runner.run_raw(&["stash", "pop", "--index", &target])?;
        if first.success {
            return Ok(());
        }
        if first.stderr.contains("without --index") {
            runner
                .run(&["stash", "pop", &target])
                .wrap_err_with(|| format!("Failed to pop stash '{}'", stash.label))?;
            return Ok(());
        }
        Err(eyre!(
            "Failed to pop stash '{}' (it is kept): {}",
            stash.label,
            first.stderr.trim()
        ))
    }

    /// Check out `branch`. A branch that exists only on a remote gets a local
    /// tracking branch. A dirty tree is handled per `on_dirty`.
    pub fn switch(&self, branch: &str, on_dirty: OnDirty) -> eyre::Result<SwitchOutcome> {
        self.switch_to(SwitchTarget::Branch(branch), on_dirty)
    }

    /// Detach HEAD at `commit` (a sha or any revision). This restores a checkout that was
    /// on a detached HEAD before, which [`switch`](Self::switch) cannot express. A dirty tree
    /// is handled per `on_dirty`; `already_on_branch` is set when HEAD is already detached there.
    pub fn switch_detached(&self, commit: &str, on_dirty: OnDirty) -> eyre::Result<SwitchOutcome> {
        self.switch_to(SwitchTarget::Commit(commit), on_dirty)
    }

    fn switch_to(
        &self,
        target: SwitchTarget<'_>,
        on_dirty: OnDirty,
    ) -> eyre::Result<SwitchOutcome> {
        let status = self.status()?;
        let mut outcome = SwitchOutcome {
            previous_branch: status.branch.clone(),
            previous_commit: status.head.clone(),
            stash: None,
            created_tracking_branch: false,
            already_on_branch: false,
        };

        // Decide how to switch before touching the tree, so a bad name never stashes.
        let (name, args): (String, Vec<String>) = match target {
            SwitchTarget::Branch(branch) => {
                if status.branch.as_deref() == Some(branch) {
                    outcome.already_on_branch = true;
                    return Ok(outcome);
                }

                let existing = self.logical_branches()?;
                let logical = existing
                    .iter()
                    .find(|b| b.name == branch)
                    .ok_or_else(|| eyre!("Branch '{branch}' not found locally or on any remote"))?;
                let args = if logical.is_local() {
                    vec!["switch".into(), "--".into(), branch.into()]
                } else {
                    match logical.remotes.as_slice() {
                        [only] => vec![
                            "switch".into(),
                            "--track".into(),
                            "--".into(),
                            only.refname.clone(),
                        ],
                        many => {
                            let names: Vec<_> = many.iter().map(|r| r.refname.as_str()).collect();
                            return Err(eyre!(
                                "Branch '{branch}' exists on several remotes ({}); cannot pick one",
                                names.join(", ")
                            ));
                        }
                    }
                };
                outcome.created_tracking_branch = !logical.is_local();
                (format!("'{branch}'"), args)
            }
            SwitchTarget::Commit(commit) => {
                let sha = self.rev_parse(commit)?;
                if status.detached && status.head.as_deref() == Some(sha.as_str()) {
                    outcome.already_on_branch = true;
                    return Ok(outcome);
                }
                (
                    format!("detached {}", short_sha(&sha)),
                    vec!["switch".into(), "--detach".into(), "--".into(), sha],
                )
            }
        };

        if !status.is_clean() {
            let label = match on_dirty {
                OnDirty::Abort => {
                    return Err(eyre!(
                        "Working tree of {} has uncommitted changes; refusing to switch to {name}",
                        self.path().display()
                    ));
                }
                OnDirty::StashLabelled(label) => label,
                OnDirty::Stash => {
                    format!("de: before switching {} to {name}", status.head_label())
                }
            };
            outcome.stash = self.stash_push(&label)?;
        }

        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Err(err) = self.runner().run(&arg_refs) {
            outcome.created_tracking_branch = false;
            // Put the working tree back the way it was found.
            if let Some(stash) = &outcome.stash
                && let Err(restore) = self.stash_pop(stash)
            {
                return Err(err.wrap_err(format!(
                    "Failed to switch to {name}; changes remain in stash '{}' ({restore})",
                    stash.label
                )));
            }
            return Err(err.wrap_err(format!("Failed to switch to {name}")));
        }

        Ok(outcome)
    }

    /// Add a worktree at `path` checked out on `branch`. A remote-only branch
    /// gets a local tracking branch. Fails if the branch is already checked out elsewhere.
    pub fn worktree_add(&self, path: &Path, branch: &str) -> eyre::Result<()> {
        let path_str = path
            .to_str()
            .ok_or_else(|| eyre!("Worktree path {} is not valid UTF-8", path.display()))?;
        let logical = self
            .logical_branches()?
            .into_iter()
            .find(|b| b.name == branch)
            .ok_or_else(|| eyre!("Branch '{branch}' not found locally or on any remote"))?;

        let runner = self.runner();
        if logical.is_local() {
            runner.run(&["worktree", "add", "--", path_str, branch])
        } else {
            let remote = logical
                .remotes
                .iter()
                .find(|r| r.kind == BranchKind::Remote)
                .ok_or_else(|| eyre!("Branch '{branch}' has no ref"))?;
            runner.run(&[
                "worktree",
                "add",
                "--track",
                "-b",
                branch,
                "--",
                path_str,
                &remote.refname,
            ])
        }
        .wrap_err_with(|| format!("Failed to add worktree for '{branch}'"))?;
        Ok(())
    }

    /// Remove a worktree. Refuses (does not force) if it has uncommitted changes.
    pub fn worktree_remove(&self, path: &Path) -> eyre::Result<()> {
        let path_str = path
            .to_str()
            .ok_or_else(|| eyre!("Worktree path {} is not valid UTF-8", path.display()))?;
        self.runner()
            .run(&["worktree", "remove", path_str])
            .wrap_err_with(|| format!("Failed to remove worktree {}", path.display()))?;
        Ok(())
    }

    /// Merge `branch` into whatever is checked out in `worktree`, always
    /// creating a merge commit (`--no-ff`) so there is one commit to record.
    /// A conflicting merge is aborted and reported, leaving the worktree clean.
    pub fn merge_in_worktree(&self, worktree: &Path, branch: &str) -> eyre::Result<MergeOutcome> {
        GitRepo::open(worktree)?.merge(branch)
    }

    /// See [`merge_in_worktree`](Self::merge_in_worktree); operates on this checkout.
    pub fn merge(&self, branch: &str) -> eyre::Result<MergeOutcome> {
        let before = self.status()?;
        if !before.is_clean() {
            return Err(eyre!(
                "Refusing to merge in {}: working tree is not clean",
                self.path().display()
            ));
        }
        let head_before = before.head;

        let runner = self.runner();
        let merged = runner.run_raw(&["merge", "--no-ff", "--no-edit", "--", branch])?;

        if merged.success {
            let head_after = self.rev_parse("HEAD")?;
            return Ok(if head_before.as_deref() == Some(head_after.as_str()) {
                MergeOutcome::UpToDate
            } else {
                MergeOutcome::Merged { commit: head_after }
            });
        }

        let unmerged = runner.run(&["diff", "--name-only", "--diff-filter=U"])?;
        let mut files: Vec<String> = unmerged.lines().map(String::from).collect();
        files.sort();

        // A merge in progress (conflicts, or a hook failure) must not be left behind.
        if runner
            .run_raw(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])?
            .success
        {
            runner
                .run(&["merge", "--abort"])
                .wrap_err("Failed to abort the merge")?;
        }

        if files.is_empty() {
            return Err(eyre!(
                "Failed to merge '{branch}' in {}: {}",
                self.path().display(),
                merged.stderr.trim()
            ));
        }
        Ok(MergeOutcome::Conflict { files })
    }
}
