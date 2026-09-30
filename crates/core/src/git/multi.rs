//! Working over several repositories without one bad repo spoiling the rest.

use std::path::Path;

use super::repo::{GitRepo, RepoStatus};

/// Status of every repo; a repo that cannot be read yields its own error.
pub fn status_all<N: Clone, P: AsRef<Path>>(
    repos: &[(N, P)],
) -> Vec<(N, eyre::Result<RepoStatus>)> {
    repos
        .iter()
        .map(|(name, path)| {
            let status = GitRepo::open(path.as_ref()).and_then(|repo| repo.status());
            (name.clone(), status)
        })
        .collect()
}

/// A repo that would lose work if torn down.
#[derive(Debug, Clone)]
pub struct AtRisk<N> {
    pub name: N,
    pub status: RepoStatus,
}

/// Outcome of checking a set of repos for uncommitted or unpushed work.
#[derive(Debug, Clone)]
pub struct RiskReport<N> {
    pub at_risk: Vec<AtRisk<N>>,
    /// Repos that could not be read (not a git repo, missing directory, ...).
    pub unreadable: Vec<(N, String)>,
}

impl<N> RiskReport<N> {
    pub fn is_safe(&self) -> bool {
        self.at_risk.is_empty()
    }
}

/// Split status results into repos with uncommitted/unpushed work and repos
/// that could not be read. Unreadable repos never count as at risk.
pub fn assess_risks<N>(results: Vec<(N, eyre::Result<RepoStatus>)>) -> RiskReport<N> {
    let mut report = RiskReport {
        at_risk: Vec::new(),
        unreadable: Vec::new(),
    };

    for (name, result) in results {
        match result {
            Ok(status) if status.has_uncommitted_or_unpushed() => {
                report.at_risk.push(AtRisk { name, status });
            }
            Ok(_) => {}
            Err(err) => report.unreadable.push((name, format!("{err:#}"))),
        }
    }

    report
}
