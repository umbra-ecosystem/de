//! The write gateway: the only code that writes to Jira, Bitbucket or a remote git branch.
//!
//! # Guarantees
//!
//! - **Typed actions.** Everything that can leave the machine is a variant of [`Action`].
//! - **Draft, confirm, execute, enforced by types.** [`Gateway::draft`] renders an exact
//!   [`ActionPreview`]; only [`Draft::confirm`] yields a [`Confirmed`]; [`Gateway::execute`]
//!   accepts nothing else, consumes it (one use) and re-checks a hash of the payload, so a
//!   payload changed after the preview is rejected. There is no bypass flag.
//! - **Audited.** An `attempted` entry with the full payload is appended *before* acting and
//!   the outcome after. If the attempted entry cannot be written, nothing is done.
//! - **The only holder of writers.** Nothing else builds a `TicketWriter` or
//!   `CodeHostWriter`; a test scans the sources to keep it so.
//!
//! # Audit entries
//!
//! Names are the constants in [`actions`]. Attempt entries are `<name>.attempted` with
//! outcome `skipped` (the audit outcome enum has no "pending") and `details.phase =
//! "attempted"`; outcome entries use `<name>` itself with `success` or `failure`.

mod action;
mod push;
#[cfg(test)]
mod tests;

use serde_json::{Value, json};

use crate::{
    config::Config,
    domain::AuditOutcome,
    providers::{
        CodeHostWriter, PipelineRun, PrComment, ProviderError, ProviderResult, RemoteComment,
        TicketWriter,
        registry::{build_code_host_writer, build_ticket_writer},
    },
    store::{
        Store,
        audit::{self, NewAuditEntry},
    },
};

pub use action::{
    Action, ActionPreview, CommitLine, Confirmed, Draft, PreviewSection, PushUat, PushUatRepo, Risk,
};
pub use push::{PushReport, PushResult, RepoPush};

/// Names of the audit entries the gateway writes.
pub mod actions {
    pub const PUSH_UAT: &str = "git.push_uat";
    pub const PUSH_UAT_REPO: &str = "git.push_uat.repo";
    pub const JIRA_COMMENT: &str = "jira.comment";
    pub const JIRA_TRANSITION: &str = "jira.transition";
    pub const PR_COMMENT: &str = "bitbucket.pr_comment";
    pub const PR_APPROVE: &str = "bitbucket.pr_approve";
    pub const PR_REQUEST_CHANGES: &str = "bitbucket.pr_request_changes";
    pub const PIPELINE_TRIGGER: &str = "bitbucket.pipeline_trigger";
    pub const PIPELINE_RERUN: &str = "bitbucket.pipeline_rerun";
    /// Appended to an action name for the entry written before acting.
    pub const ATTEMPTED_SUFFIX: &str = ".attempted";
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// The payload differs from what was previewed and confirmed.
    #[error(
        "the payload changed after it was previewed; nothing was sent (confirmed {expected}, now {actual})"
    )]
    PayloadChanged { expected: String, actual: String },
    /// The audit entry could not be written, so nothing was done.
    #[error("could not write the audit entry, so nothing was sent: {0:#}")]
    Audit(eyre::Report),
    /// The adapter for this system is not available (not installed, not built).
    #[error("cannot write: {0}")]
    WriterUnavailable(ProviderError),
    /// The provider rejected or failed the write (recorded in the audit log).
    #[error("the write failed: {0}")]
    Provider(ProviderError),
    /// A condition that must hold before writing did not.
    #[error("{0}")]
    Precondition(String),
    #[error(transparent)]
    Other(#[from] eyre::Report),
}

/// What an executed action returned.
#[derive(Debug)]
pub enum Outcome {
    Pushed(PushReport),
    JiraComment(RemoteComment),
    Transitioned,
    PrComment(PrComment),
    Approved,
    ChangesRequested,
    Pipeline(PipelineRun),
}

#[derive(Debug)]
pub struct Executed {
    pub draft_id: String,
    pub outcome: Outcome,
    /// The action happened but an audit entry after it could not be written.
    pub audit_warnings: Vec<String>,
}

/// The write gateway. Construct with [`Gateway::from_config`] (the real adapters) or
/// [`Gateway::with_writers`] (tests).
pub struct Gateway<'a> {
    state: &'a Store,
    ticket_writer: ProviderResult<Box<dyn TicketWriter>>,
    code_writer: ProviderResult<Box<dyn CodeHostWriter>>,
}

impl<'a> Gateway<'a> {
    /// Builds the writers through the provider registry. A writer that cannot be built is
    /// remembered and reported when an action needs it.
    pub fn from_config(state: &'a Store, config: &Config) -> Self {
        Self::with_writers(
            state,
            build_ticket_writer(config),
            build_code_host_writer(config),
        )
    }

    pub fn with_writers(
        state: &'a Store,
        ticket_writer: ProviderResult<Box<dyn TicketWriter>>,
        code_writer: ProviderResult<Box<dyn CodeHostWriter>>,
    ) -> Self {
        Self {
            state,
            ticket_writer,
            code_writer,
        }
    }

    pub fn state(&self) -> &Store {
        self.state
    }

    /// Renders the exact preview of `action`. Nothing is sent or recorded.
    pub fn draft(&self, action: Action) -> Draft {
        Draft::new(action, Value::Null)
    }

    /// Like [`draft`](Self::draft), with the facts that motivate the action (recorded in the
    /// audit log).
    pub fn draft_because(&self, action: Action, facts: Value) -> Draft {
        Draft::new(action, facts)
    }

    /// Fails clearly when the writer `action` needs is not available, so a caller can say so
    /// before asking anyone to confirm.
    pub fn ensure_available(&self, action: &Action) -> Result<(), GatewayError> {
        let unavailable = match action {
            Action::PushUat(_) => None,
            Action::PostJiraComment { .. } | Action::TransitionJira { .. } => {
                self.ticket_writer.as_ref().err()
            }
            _ => self.code_writer.as_ref().err(),
        };
        match unavailable {
            Some(e) => Err(GatewayError::WriterUnavailable(e.clone())),
            None => Ok(()),
        }
    }

    fn append(&self, entry: NewAuditEntry) -> eyre::Result<i64> {
        audit::append(self.state, &entry)
    }

    /// Executes a confirmed action: verify the payload, audit the attempt, act, audit the
    /// outcome.
    pub fn execute(&self, confirmed: Confirmed, now: i64) -> Result<Executed, GatewayError> {
        let actual = confirmed.action.hash();
        if actual != confirmed.payload_hash {
            return Err(GatewayError::PayloadChanged {
                expected: confirmed.payload_hash.clone(),
                actual,
            });
        }
        self.ensure_available(&confirmed.action)?;

        let action = &confirmed.action;
        let name = action.audit_name();
        let ticket = action.ticket().cloned();
        let payload: Value = serde_json::from_str(&action.canonical()).unwrap_or(Value::Null);

        // Never act unaudited: if this fails, nothing below runs.
        self.append(NewAuditEntry {
            at: now,
            action: format!("{name}{}", actions::ATTEMPTED_SUFFIX),
            ticket: ticket.clone(),
            repo: None,
            details: json!({
                "phase": "attempted",
                "gateway": confirmed.facts_with_id(),
                "payload": payload,
            }),
            outcome: AuditOutcome::Skipped,
        })
        .map_err(GatewayError::Audit)?;

        let mut warnings = Vec::new();
        let result = self.run(&confirmed, now, &mut warnings);

        let (outcome_kind, details) = match &result {
            Ok(Outcome::Pushed(report)) if !report.all_done() => (
                AuditOutcome::Failure,
                describe(&Outcome::Pushed(report.clone())),
            ),
            Ok(outcome) => (AuditOutcome::Success, describe(outcome)),
            Err(e) => (AuditOutcome::Failure, json!({ "error": e.to_string() })),
        };
        let mut details = details;
        details["phase"] = json!("outcome");
        details["gateway"] = confirmed.facts_with_id();
        if let Err(e) = self.append(NewAuditEntry {
            at: now,
            action: name.into(),
            ticket,
            repo: None,
            details,
            outcome: outcome_kind,
        }) {
            warnings.push(format!("the outcome could not be audited: {e:#}"));
        }

        Ok(Executed {
            draft_id: confirmed.draft_id.clone(),
            outcome: result?,
            audit_warnings: warnings,
        })
    }

    fn run(
        &self,
        confirmed: &Confirmed,
        now: i64,
        warnings: &mut Vec<String>,
    ) -> Result<Outcome, GatewayError> {
        let unavailable = |e: &ProviderError| GatewayError::WriterUnavailable(e.clone());
        match &confirmed.action {
            Action::PushUat(push) => Ok(Outcome::Pushed(self.push_uat(push, now, warnings)?)),
            Action::PostJiraComment { ticket, body } => {
                let writer = self.ticket_writer.as_ref().map_err(unavailable)?;
                writer
                    .add_comment(ticket, body)
                    .map(Outcome::JiraComment)
                    .map_err(GatewayError::Provider)
            }
            Action::TransitionJira { ticket, to_status } => {
                let writer = self.ticket_writer.as_ref().map_err(unavailable)?;
                writer
                    .transition(ticket, to_status)
                    .map(|()| Outcome::Transitioned)
                    .map_err(GatewayError::Provider)
            }
            Action::PostPrComment { repo, pr, comment } => {
                let writer = self.code_writer.as_ref().map_err(unavailable)?;
                writer
                    .add_pr_comment(repo, *pr, comment)
                    .map(Outcome::PrComment)
                    .map_err(GatewayError::Provider)
            }
            Action::ApprovePr { repo, pr } => {
                let writer = self.code_writer.as_ref().map_err(unavailable)?;
                writer
                    .approve(repo, *pr)
                    .map(|()| Outcome::Approved)
                    .map_err(GatewayError::Provider)
            }
            Action::RequestChanges { repo, pr, body } => {
                let writer = self.code_writer.as_ref().map_err(unavailable)?;
                writer
                    .request_changes(repo, *pr, body)
                    .map(|()| Outcome::ChangesRequested)
                    .map_err(GatewayError::Provider)
            }
            Action::TriggerPipeline { repo, spec } => {
                let writer = self.code_writer.as_ref().map_err(unavailable)?;
                writer
                    .trigger_pipeline(repo, spec)
                    .map(Outcome::Pipeline)
                    .map_err(GatewayError::Provider)
            }
            Action::RerunPipeline { repo, run_id } => {
                let writer = self.code_writer.as_ref().map_err(unavailable)?;
                writer
                    .rerun_pipeline(repo, run_id)
                    .map(Outcome::Pipeline)
                    .map_err(GatewayError::Provider)
            }
        }
    }
}

fn describe(outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Pushed(report) => json!({
            "repos": report.repos.iter().map(|r| json!({ "repo": r.repo, "result": r.result.describe() })).collect::<Vec<_>>(),
        }),
        Outcome::JiraComment(c) => json!({ "remote_id": c.id }),
        Outcome::PrComment(c) => json!({ "remote_id": c.id }),
        Outcome::Pipeline(r) => json!({ "run_id": r.id, "number": r.number }),
        Outcome::Transitioned | Outcome::Approved | Outcome::ChangesRequested => json!({}),
    }
}
