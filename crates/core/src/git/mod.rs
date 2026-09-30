//! Git layer: structured status, ticket-key branch matching, diffs from
//! objects, and safe mutations.
//!
//! Reads go through `git2`. Anything that changes a repository or touches a
//! network runs the `git` binary (see [`GitRunner`]) so the user's SSH agent,
//! credential helpers and hooks apply.

mod diff;
mod keymatch;
mod multi;
mod ops;
mod repo;
mod runner;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod testutil;

pub use diff::{DiffLine, FileDiff, FileStatus, Hunk, LineKind, RepoDiff};
pub use keymatch::name_contains_key;
pub use multi::{AtRisk, RiskReport, assess_risks, status_all};
pub use ops::{FastForward, MergeOutcome, OnDirty, StashEntry, StashRef, SwitchOutcome};
pub use repo::{
    BaseBranch, BranchInfo, BranchKind, BranchMatch, CommitInfo, GitRepo, LogicalBranch,
    RepoStatus, group_branches, match_branches, pick_base, short_sha,
};
pub use runner::{GitCommandError, GitOutput, GitRunner};
