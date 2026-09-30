//! Making sure the test overlay never reaches `uat`.
//!
//! [`check_range_for_overlay`] looks at every commit about to be pushed;
//! [`working_tree_has_overlay`] looks at a checkout, so callers can refuse to integrate from
//! a tree that still carries the overlay.

use std::path::Path;

use eyre::Context;

use super::composer::{Finding, LeakKind, scan_composer_json, scan_composer_lock};
use crate::git::{FileDiff, GitRepo, LineKind, short_sha};

/// One overlay-looking line found in a commit that is about to be pushed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeakedChange {
    pub commit: String,
    pub summary: String,
    /// Path of the file within the repo.
    pub file: String,
    /// Line number in the file as of that commit.
    pub line: Option<u32>,
    pub text: String,
    pub kind: LeakKind,
}

/// The commits about to be pushed carry the test overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayLeak {
    pub leaks: Vec<LeakedChange>,
}

impl std::fmt::Display for OverlayLeak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the test overlay is in the commits to push ({} line{}):",
            self.leaks.len(),
            if self.leaks.len() == 1 { "" } else { "s" }
        )?;
        for leak in &self.leaks {
            write!(
                f,
                "\n  {} \"{}\": {}:{} {} ({})",
                short_sha(&leak.commit),
                leak.summary,
                leak.file,
                leak.line.map_or_else(String::new, |l| l.to_string()),
                leak.text,
                leak.kind
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for OverlayLeak {}

/// Why the guard did not pass.
#[derive(Debug, thiserror::Error)]
pub enum OverlayGuardError {
    #[error(transparent)]
    Leak(OverlayLeak),
    /// The history could not be read. The guard fails closed: nothing was verified.
    #[error("could not verify the commits: {0:#}")]
    Unreadable(eyre::Report),
}

fn is_composer_file(path: &str, name: &str) -> bool {
    path == name || path.ends_with(&format!("/{name}"))
}

/// Scans the lines added by every commit in `base..head` to `composer.json` and
/// `composer.lock` for overlay signatures (see [`LeakKind`]). `packages` are the composer
/// package names the repo's config maps as overlay packages.
///
/// Each commit is checked on its own, so a leak added in a middle commit and removed by a
/// later one is still reported: the commit is pushed either way. A merge commit is checked
/// for the lines it adds relative to every parent (what its conflict resolution wrote); a
/// root commit against the empty tree; renames are not followed, so a moved file counts as new.
pub fn check_range_for_overlay(
    repo: &GitRepo,
    base: &str,
    head: &str,
    packages: &[&str],
) -> Result<(), OverlayGuardError> {
    let leaks = scan_range(repo, base, head, packages).map_err(OverlayGuardError::Unreadable)?;
    if leaks.is_empty() {
        Ok(())
    } else {
        Err(OverlayGuardError::Leak(OverlayLeak { leaks }))
    }
}

fn scan_range(
    repo: &GitRepo,
    base: &str,
    head: &str,
    packages: &[&str],
) -> eyre::Result<Vec<LeakedChange>> {
    let mut leaks = Vec::new();

    // Oldest first, so the report reads in the order the commits were made.
    let mut commits = repo.commits_not_in(base, head)?;
    commits.reverse();

    for commit in commits {
        // One diff per parent, without rename detection: a leak moved into place by a rename,
        // or written into a root commit or a merge's conflict resolution, still shows up as
        // added lines.
        let per_parent = repo
            .commit_diffs(&commit.sha)
            .wrap_err_with(|| format!("Failed to diff commit {}", short_sha(&commit.sha)))?;

        let mut per_parent_findings = per_parent.iter().map(|files| scan_files(files, packages));
        let mut findings = per_parent_findings.next().unwrap_or_default();
        // A merge introduces only what differs from every parent; the rest is the history it
        // joins, checked commit by commit.
        for other in per_parent_findings {
            findings.retain(|(file, f)| {
                other
                    .iter()
                    .any(|(f2, o)| f2 == file && o.kind == f.kind && o.text == f.text)
            });
        }

        leaks.extend(findings.into_iter().map(|(file, f)| LeakedChange {
            commit: commit.sha.clone(),
            summary: commit.summary.clone(),
            file,
            line: f.line,
            text: f.text,
            kind: f.kind,
        }));
    }

    Ok(leaks)
}

/// Overlay signatures in the lines a set of file diffs adds to composer files.
fn scan_files(files: &[FileDiff], packages: &[&str]) -> Vec<(String, Finding)> {
    let mut found = Vec::new();
    for file in files {
        let is_json = is_composer_file(&file.path, "composer.json");
        let is_lock = is_composer_file(&file.path, "composer.lock");
        if !is_json && !is_lock {
            continue;
        }

        let added: Vec<(Option<u32>, &str)> = file
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == LineKind::Added)
            .map(|l| (l.new_lineno, l.content.as_str()))
            .collect();
        let findings = if is_json {
            scan_composer_json(&added, packages)
        } else {
            scan_composer_lock(&added)
        };
        found.extend(findings.into_iter().map(|f| (file.path.clone(), f)));
    }
    found
}

/// Overlay signatures in the `composer.json` and `composer.lock` at the root of the checkout
/// at `dir` (empty when it is clean, or has neither file).
pub fn working_tree_overlay(dir: &Path, packages: &[&str]) -> eyre::Result<Vec<Finding>> {
    let mut findings = Vec::new();

    for (name, is_json) in [("composer.json", true), ("composer.lock", false)] {
        let path = dir.join(name);
        let text = match std::fs::read(&path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(
                    eyre::Report::new(e).wrap_err(format!("Failed to read {}", path.display()))
                );
            }
        };
        let lines: Vec<(Option<u32>, &str)> = text
            .lines()
            .enumerate()
            .map(|(i, l)| (Some(i as u32 + 1), l))
            .collect();
        findings.extend(if is_json {
            scan_composer_json(&lines, packages)
        } else {
            scan_composer_lock(&lines)
        });
    }

    Ok(findings)
}

/// Whether the checkout at `dir` carries the test overlay in its composer files right now.
pub fn working_tree_has_overlay(dir: &Path, packages: &[&str]) -> eyre::Result<bool> {
    Ok(!working_tree_overlay(dir, packages)?.is_empty())
}
