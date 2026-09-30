//! The sync engine: refreshes the local cache (`cache.db`) from the providers.
//!
//! # Guarantees
//!
//! - **Read-only by construction.** Every entry point accepts only [`TicketProvider`] and
//!   [`CodeHost`] (the read traits). A writer cannot be passed in, so a sync cannot write to
//!   Jira or Bitbucket. Tests assert this against the fakes' call log.
//! - **Failure isolation.** [`sync_all`] runs every source (Jira, and each hosted repo)
//!   independently. A failing source never aborts another and never deletes cached data.
//! - **No deletion on failure.** Stale rows (a PR that vanished, a ticket that left the
//!   pool, a comment that was deleted) are removed only after the fetch that proves it
//!   succeeded completely.
//! - **Offline is a state, not an error.** [`SyncReport`] says per source whether it synced,
//!   was skipped as fresh, or failed and why (`offline`, `not logged in`, ...), so the UI
//!   can show it while everything cached stays usable.
//! - **No hidden clock.** All times come from [`SyncContext::now`].

mod code_host;
mod hosted;
mod jira;
pub mod kind;
mod report;

#[cfg(test)]
mod tests;

pub use code_host::{RepoCommit, sync_code_host, uat_commits};
pub use hosted::{HostedRepo, is_production_branch};
pub use jira::sync_jira;
pub use kind::{DerivedKind, KindReason, derive_kind, ticket_kind};
pub use report::{Problem, SourceOutcome, SourceReport, SyncCounts, SyncReport, SyncSource};

use crate::config::Config;
use crate::providers::{CodeHost, ProviderError, TicketProvider};
use crate::store::{Store, sync_state};

/// A source is skipped when it synced completely less than this many seconds ago (unless
/// forced). Callers may pass their own; this is the default of `de sync`.
pub const DEFAULT_MIN_INTERVAL: i64 = 60;

/// Everything a sync needs besides the providers.
pub struct SyncContext<'a> {
    /// `state.db`: read for the tracked tickets, links and recorded `uat` merges.
    pub state: &'a Store,
    /// `cache.db`: written.
    pub cache: &'a Store,
    pub config: &'a Config,
    /// The current time, unix seconds.
    pub now: i64,
    /// Sync even sources that synced within `min_interval`.
    pub force: bool,
    /// Seconds a completely successful sync stays fresh.
    pub min_interval: i64,
}

/// Runs the sources selected by `only` (all when `None`) independently and reports each.
///
/// A provider that could not be built is passed as `Err(&error)` (the shape
/// `Result<Box<dyn P>, E>::as_deref()` produces): its source is reported as failed with that
/// error, and the other source still runs. `uat_commits` are `(hosting repo, commit)` pairs
/// whose pipelines to look up; see [`uat_commits`] for building them from `state.db`.
pub fn sync_all(
    ctx: &SyncContext<'_>,
    jira: Result<&dyn TicketProvider, &ProviderError>,
    code_host: Result<&dyn CodeHost, &ProviderError>,
    repos: &[HostedRepo],
    uat_commits: &[RepoCommit],
    only: Option<SyncSource>,
) -> SyncReport {
    let mut report = SyncReport::default();

    if only.is_none_or(|s| s == SyncSource::Jira) {
        report.sources.push(match jira {
            Ok(provider) => sync_jira(ctx, provider),
            Err(error) => report::provider_unavailable(
                ctx,
                SyncSource::Jira,
                sync_state::JIRA.into(),
                error,
                ctx.config.jira.is_none(),
            ),
        });
    }

    if only.is_none_or(|s| s == SyncSource::Bitbucket) {
        if repos.is_empty() {
            report.sources.push(SourceReport::not_configured(
                SyncSource::Bitbucket,
                "bitbucket",
                "no project has a [hosting] section",
            ));
        } else {
            match code_host {
                Ok(host) => report
                    .sources
                    .extend(sync_code_host(ctx, host, repos, uat_commits)),
                Err(error) => {
                    for repo in repos {
                        report.sources.push(report::provider_unavailable(
                            ctx,
                            SyncSource::Bitbucket,
                            sync_state::code_host_source(&repo.repo),
                            error,
                            false,
                        ));
                    }
                }
            }
        }
    }

    report
}
