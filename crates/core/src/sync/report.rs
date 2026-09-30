//! What a sync did, per source, and the machinery that turns a run into a report.

use crate::providers::{ProviderError, ProviderErrorKind};
use crate::store::sync_state;

use super::SyncContext;

/// The systems sync talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncSource {
    Jira,
    Bitbucket,
}

impl SyncSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncSource::Jira => "jira",
            SyncSource::Bitbucket => "bitbucket",
        }
    }
}

/// One thing that went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// `Network` is "offline", `NotAuthenticated` is "not logged in", `Local` is a database
    /// or configuration failure; the rest are per-call provider problems.
    pub kind: ProviderErrorKind,
    pub message: String,
    /// What was being fetched (`PROJ-1`, `open PRs`, `pipeline for abc1234`), when known.
    pub subject: Option<String>,
}

impl Problem {
    pub(crate) fn from_provider(subject: Option<String>, error: &ProviderError) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
            subject,
        }
    }

    pub(crate) fn local(error: &eyre::Report) -> Self {
        Self {
            kind: ProviderErrorKind::Local,
            message: format!("{error:#}"),
            subject: None,
        }
    }
}

/// How one source's sync ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOutcome {
    /// Everything fetched; stale rows removed.
    Synced,
    /// The source could be reached and most things synced, but some fetches failed for
    /// reasons specific to them. Cached data for the failed items is untouched.
    Partial(Vec<Problem>),
    /// The source could not be synced (offline, not logged in, adapter missing, local error).
    /// The cache is untouched from the point of failure on.
    Failed(Problem),
    /// Synced completely within `min_interval`; nothing was fetched.
    Skipped { last_ok_at: i64 },
    /// Nothing to sync because it is not configured (a reason, not an error).
    NotConfigured(String),
}

/// How much was written to the cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncCounts {
    pub tickets: usize,
    pub ticket_comments: usize,
    pub prs: usize,
    pub pr_comments: usize,
    pub pipelines: usize,
    /// Stale cache rows removed.
    pub removed: usize,
}

/// The result for one source: `jira`, or one hosted repo (`bitbucket:acme/web`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReport {
    pub system: SyncSource,
    /// The `sync_state` key.
    pub source: String,
    pub outcome: SourceOutcome,
    pub counts: SyncCounts,
    /// Non-error remarks (a setting that is missing so part of the sync was skipped).
    pub notes: Vec<String>,
}

impl SourceReport {
    pub(crate) fn not_configured(system: SyncSource, source: &str, reason: &str) -> Self {
        Self {
            system,
            source: source.into(),
            outcome: SourceOutcome::NotConfigured(reason.into()),
            counts: SyncCounts::default(),
            notes: Vec::new(),
        }
    }

    /// Whether the source ended without any failure (synced, skipped or not configured).
    pub fn is_ok(&self) -> bool {
        !matches!(
            self.outcome,
            SourceOutcome::Failed(_) | SourceOutcome::Partial(_)
        )
    }

    /// The first problem, if any.
    pub fn problem(&self) -> Option<&Problem> {
        match &self.outcome {
            SourceOutcome::Failed(p) => Some(p),
            SourceOutcome::Partial(ps) => ps.first(),
            _ => None,
        }
    }
}

/// The result of a whole sync.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub sources: Vec<SourceReport>,
}

impl SyncReport {
    /// Whether every source ended without failure.
    pub fn is_ok(&self) -> bool {
        self.sources.iter().all(SourceReport::is_ok)
    }

    /// The reports that failed outright or partially.
    pub fn failures(&self) -> impl Iterator<Item = &SourceReport> {
        self.sources.iter().filter(|s| !s.is_ok())
    }
}

/// Accumulates what a source's sync does; handed to the per-source bodies.
#[derive(Debug, Default)]
pub(crate) struct Collector {
    pub counts: SyncCounts,
    pub problems: Vec<Problem>,
    pub notes: Vec<String>,
}

/// Aborts the rest of a source: its error is the outcome.
pub(crate) type Abort = Problem;
pub(crate) type Step = Result<(), Abort>;

impl Collector {
    /// Handles a provider error for one fetch. An environmental error (offline, not logged
    /// in, not installed) aborts the source (`Err`); anything else is recorded and the
    /// caller carries on with the next item (`Ok`).
    pub fn handle(&mut self, subject: impl Into<String>, error: &ProviderError) -> Step {
        let problem = Problem::from_provider(Some(subject.into()), error);
        if error.is_environmental() {
            Err(problem)
        } else {
            self.problems.push(problem);
            Ok(())
        }
    }
}

pub(crate) fn local(error: eyre::Report) -> Abort {
    Problem::local(&error)
}

fn describe(problem: &Problem) -> String {
    match &problem.subject {
        Some(s) => format!("{}: {s}: {}", problem.kind, problem.message),
        None => format!("{}: {}", problem.kind, problem.message),
    }
}

/// Runs one source: skips it when fresh, runs `body`, records the outcome in `sync_state`
/// and builds the report. A failure to record is logged, never fatal.
pub(crate) fn run_source(
    ctx: &SyncContext<'_>,
    system: SyncSource,
    source: String,
    body: impl FnOnce(&mut Collector) -> Step,
) -> SourceReport {
    let existing = sync_state::get(ctx.cache, &source);
    if !ctx.force
        && let Ok(Some(state)) = &existing
        && state.is_fresh(ctx.now, ctx.min_interval)
        && let Some(last_ok_at) = state.last_ok_at
    {
        return SourceReport {
            system,
            source,
            outcome: SourceOutcome::Skipped { last_ok_at },
            counts: SyncCounts::default(),
            notes: Vec::new(),
        };
    }

    let mut collector = Collector::default();
    let result = body(&mut collector);
    let Collector {
        counts,
        problems,
        notes,
    } = collector;

    let outcome = match result {
        Err(problem) => SourceOutcome::Failed(problem),
        Ok(()) if problems.is_empty() => SourceOutcome::Synced,
        Ok(()) => SourceOutcome::Partial(problems),
    };

    let recorded = match &outcome {
        SourceOutcome::Synced => sync_state::record_ok(ctx.cache, &source, ctx.now),
        SourceOutcome::Failed(p) => {
            sync_state::record_failure(ctx.cache, &source, ctx.now, &describe(p))
        }
        SourceOutcome::Partial(ps) => {
            let text = ps.iter().map(describe).collect::<Vec<_>>().join("; ");
            sync_state::record_failure(ctx.cache, &source, ctx.now, &text)
        }
        SourceOutcome::Skipped { .. } | SourceOutcome::NotConfigured(_) => Ok(()),
    };
    if let Err(e) = recorded {
        tracing::warn!("could not record the sync state of {source}: {e:#}");
    }

    SourceReport {
        system,
        source,
        outcome,
        counts,
        notes,
    }
}

/// The report for a source whose provider could not be built. Recorded in `sync_state` so
/// a later reader sees why the source has never synced.
pub(crate) fn provider_unavailable(
    ctx: &SyncContext<'_>,
    system: SyncSource,
    source: String,
    error: &ProviderError,
    not_configured: bool,
) -> SourceReport {
    if not_configured {
        return SourceReport::not_configured(system, &source, "no [jira] section in the config");
    }
    let problem = Problem::from_provider(None, error);
    if let Err(e) = sync_state::record_failure(ctx.cache, &source, ctx.now, &describe(&problem)) {
        tracing::warn!("could not record the sync state of {source}: {e:#}");
    }
    SourceReport {
        system,
        source,
        outcome: SourceOutcome::Failed(problem),
        counts: SyncCounts::default(),
        notes: Vec::new(),
    }
}
