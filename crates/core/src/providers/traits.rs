//! The provider contracts. Adapters (CLI wrappers or REST clients) implement these; sync and
//! the write gateway consume them.
//!
//! # Rules every implementation follows
//!
//! - **Blocking and `Send`.** No async; an implementation may hold a `ProcessRunner` or an
//!   HTTP client, but everything it owns must be `Send` so it can move to a sync thread.
//!   Methods take `&self`; use interior mutability for any cache an adapter keeps.
//! - **Structured output only.** Adapters parse JSON/YAML the tool prints, never prose. A
//!   shape they do not understand is [`ProviderError::Parse`], never a guess.
//! - **`de` never handles credentials.** A missing or expired login is
//!   [`ProviderError::NotAuthenticated`]; a missing binary is [`ProviderError::NotInstalled`];
//!   a network failure is [`ProviderError::Network`]. Sync depends on these being distinct.
//! - **Pagination is the adapter's problem.** `search`, `list_prs` and `pipelines` return
//!   the complete result (up to `limit` where one exists).
//! - **Read traits never write, write traits never read.** See below.
//!
//! # Reads and writes are separate traits on purpose
//!
//! [`TicketProvider`] and [`CodeHost`] are read-only. [`TicketWriter`] and
//! [`CodeHostWriter`] are the only way to change a remote system. **Only the write gateway
//! (M5) ever holds a writer**: it records every write in the audit log first. Sync and every
//! other reader accept only `&dyn TicketProvider` / `&dyn CodeHost`, so a background sync
//! cannot write even by mistake. An adapter type typically implements both a read and a
//! write trait, but it must be constructed for each separately through
//! [`registry`](super::registry) so a reader is never a writer.

use super::error::ProviderResult;
use super::model::{
    Health, NewPrComment, PipelineFilter, PipelineRun, Pr, PrComment, PrFilter, RemoteComment,
    RemoteTicket, TriggerSpec,
};
use crate::domain::TicketKey;

/// Read access to the ticket system (Jira).
pub trait TicketProvider: Send {
    /// Reports whether the tool is installed, logged in and recent enough. Never fails:
    /// problems are the answer. May spawn a process; not for hot paths.
    fn health(&self) -> Health;

    /// Runs a JQL query and returns every match, complete (paginated internally).
    fn search(&self, jql: &str) -> ProviderResult<Vec<RemoteTicket>>;

    /// One ticket by key. A ticket that does not exist (or is not visible) is
    /// [`ProviderError::NotFound`](super::ProviderError::NotFound).
    fn get(&self, key: &TicketKey) -> ProviderResult<RemoteTicket>;

    /// All comments of a ticket, oldest first, with the account ids each one mentions.
    fn comments(&self, key: &TicketKey) -> ProviderResult<Vec<RemoteComment>>;

    /// Human-readable notes accumulated by earlier calls that succeeded but were lossy
    /// (for example a comment list the tool truncated), cleared by this call. Sync surfaces
    /// them as report notes. The default is none.
    fn take_warnings(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Read access to the code host (Bitbucket): pull requests and pipelines.
///
/// `repo` is always `workspace/slug`.
pub trait CodeHost: Send {
    fn health(&self) -> Health;

    /// PRs of `repo` matching the filter, complete. Reviewer lists may be partial here;
    /// [`CodeHost::pr`] is authoritative.
    fn list_prs(&self, repo: &str, filter: &PrFilter) -> ProviderResult<Vec<Pr>>;

    /// One PR with its full reviewer list.
    fn pr(&self, repo: &str, id: u64) -> ProviderResult<Pr>;

    /// Every comment of a PR (general and inline), oldest first.
    fn pr_comments(&self, repo: &str, id: u64) -> ProviderResult<Vec<PrComment>>;

    /// Pipeline runs of `repo`, newest first, honouring the filter. Steps may be empty;
    /// [`CodeHost::pipeline`] fills them.
    fn pipelines(&self, repo: &str, filter: &PipelineFilter) -> ProviderResult<Vec<PipelineRun>>;

    /// One run with its steps.
    fn pipeline(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun>;
}

/// Write access to the ticket system. **Held only by the write gateway.**
pub trait TicketWriter: Send {
    /// Posts a comment; returns it as Jira stored it.
    fn add_comment(&self, key: &TicketKey, body: &str) -> ProviderResult<RemoteComment>;

    /// Moves the ticket to the workflow status named `to_status` (for example
    /// `Alpha Testing`). An unavailable transition is [`ProviderError::Unsupported`](super::ProviderError::Unsupported).
    fn transition(&self, key: &TicketKey, to_status: &str) -> ProviderResult<()>;
}

/// Write access to the code host. **Held only by the write gateway.**
pub trait CodeHostWriter: Send {
    fn add_pr_comment(
        &self,
        repo: &str,
        id: u64,
        comment: &NewPrComment,
    ) -> ProviderResult<PrComment>;

    fn approve(&self, repo: &str, id: u64) -> ProviderResult<()>;

    fn request_changes(&self, repo: &str, id: u64, body: &str) -> ProviderResult<()>;

    /// Starts a pipeline run and returns it.
    fn trigger_pipeline(&self, repo: &str, spec: &TriggerSpec) -> ProviderResult<PipelineRun>;

    /// Re-runs a finished run; returns the new run.
    fn rerun_pipeline(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun>;
}
