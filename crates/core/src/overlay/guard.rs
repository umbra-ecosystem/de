//! Making sure the test overlay never reaches `uat`.
//!
//! [`check_range_for_overlay`] looks at every commit about to be pushed;
//! [`working_tree_has_overlay`] looks at a checkout, so callers can refuse to integrate from
//! a tree that still carries the overlay.

use std::path::Path;

use eyre::Context;

use super::composer::{
    Finding, LeakKind, is_wildcard_constraint, scan_composer_json, scan_composer_lock,
};
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

        // Lines can miss a leak: git may show an unchanged `"type": "path"` line as context
        // when it lines a new entry up with an old one. So the parsed documents are compared
        // too, and anything overlay-shaped that a parent did not have is a leak.
        for (file, entry) in semantic_additions(repo, &commit.sha, &per_parent, packages)? {
            if !findings
                .iter()
                .any(|(f, found)| *f == file && found.kind == entry.kind)
            {
                findings.push((file, entry));
            }
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

/// Overlay-shaped things in the composer files `commit` changed that none of its parents had.
fn semantic_additions(
    repo: &GitRepo,
    sha: &str,
    per_parent: &[Vec<FileDiff>],
    packages: &[&str],
) -> eyre::Result<Vec<(String, Finding)>> {
    let commit = repo.commit(sha)?;
    let tree = commit.tree()?;
    let parent_trees = commit
        .parents()
        .map(|p| p.tree())
        .collect::<Result<Vec<_>, _>>()?;

    let mut paths: Vec<&str> = per_parent
        .iter()
        .flatten()
        .filter(|f| {
            is_composer_file(&f.path, "composer.json") || is_composer_file(&f.path, "composer.lock")
        })
        .map(|f| f.path.as_str())
        .collect();
    paths.sort_unstable();
    paths.dedup();

    let blob = |tree: &git2::Tree<'_>, path: &str| -> Option<Vec<u8>> {
        let entry = tree.get_path(Path::new(path)).ok()?;
        Some(repo.inner().find_blob(entry.id()).ok()?.content().to_vec())
    };

    let mut found = Vec::new();
    for path in paths {
        let Some(now) = blob(&tree, path) else {
            continue;
        };
        let is_json = is_composer_file(path, "composer.json");
        let after = document_signatures(&now, is_json, packages);
        for (kind, signature) in after {
            let in_every_parent_absent = parent_trees.iter().all(|parent| {
                blob(parent, path)
                    .map(|bytes| document_signatures(&bytes, is_json, packages))
                    .is_none_or(|before| !before.iter().any(|(k, s)| *k == kind && *s == signature))
            });
            if in_every_parent_absent {
                found.push((
                    String::from(path),
                    Finding {
                        kind,
                        line: None,
                        text: signature,
                    },
                ));
            }
        }
    }
    Ok(found)
}

/// What a composer file contains that looks like the overlay, from its parsed form:
/// symlinked path repositories and wildcard constraints for `packages` in `composer.json`,
/// packages installed from a path in `composer.lock`. Empty when the file is not valid JSON.
fn document_signatures(bytes: &[u8], is_json: bool, packages: &[&str]) -> Vec<(LeakKind, String)> {
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    if is_json {
        // `repositories` is a list, or an object keyed by name.
        let repositories: Vec<&serde_json::Value> = match document.get("repositories") {
            Some(serde_json::Value::Array(list)) => list.iter().collect(),
            Some(serde_json::Value::Object(map)) => map.values().collect(),
            _ => Vec::new(),
        };
        for repository in repositories {
            let symlinked = repository
                .pointer("/options/symlink")
                .is_some_and(|v| v == &serde_json::Value::Bool(true));
            if repository.get("type").and_then(|t| t.as_str()) == Some("path") && symlinked {
                out.push((LeakKind::PathRepository, repository.to_string()));
            }
        }
        for section in ["require", "require-dev"] {
            let Some(deps) = document.get(section).and_then(|d| d.as_object()) else {
                continue;
            };
            for (package, constraint) in deps {
                let wildcard = constraint.as_str().is_some_and(is_wildcard_constraint);
                if wildcard && packages.iter().any(|p| p.eq_ignore_ascii_case(package)) {
                    out.push((
                        LeakKind::WildcardConstraint,
                        format!("{section}: \"{package}\": {constraint}"),
                    ));
                }
            }
        }
    } else {
        for section in ["packages", "packages-dev"] {
            let Some(list) = document.get(section).and_then(|l| l.as_array()) else {
                continue;
            };
            for package in list {
                if package.pointer("/dist/type").and_then(|t| t.as_str()) == Some("path") {
                    out.push((
                        LeakKind::LockPathDist,
                        package
                            .pointer("/dist")
                            .map_or_else(String::new, |d| d.to_string()),
                    ));
                }
            }
        }
    }
    out
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
