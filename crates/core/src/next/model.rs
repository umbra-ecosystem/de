//! The vocabulary of the next-action engine: suggestions, their typed actions, rule ids,
//! execution levels and the priority scheme.

use std::fmt;

use serde::Serialize;
use serde_json::Value;

use crate::{
    domain::{BaselineChoice, TicketKey},
    gateway::Action,
};

/// How much a suggestion may do on its own (GOAL.md, "Next-action engine").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLevel {
    /// Runs on its own; never changes a working tree or an external system (sync, fetch,
    /// recompute, or a plain "waiting" note).
    Automatic,
    /// Suggested; runs after a click. Local and reversible (activate, park, revert the
    /// overlay, claim, draft a comment, ...), or only shows something (open a PR).
    LocalOneClick,
    /// Goes through the write gateway: an exact preview, an explicit confirmation, an audit
    /// entry.
    ConfirmedExternal,
}

impl ExecutionLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionLevel::Automatic => "automatic",
            ExecutionLevel::LocalOneClick => "local_one_click",
            ExecutionLevel::ConfirmedExternal => "confirmed_external",
        }
    }
}

impl fmt::Display for ExecutionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which rule produced a suggestion. Each rule is a small pure function in
/// [`rules`](super::rules) whose doc comment names the flow step it comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleId {
    SyncStale,
    ClaimNew,
    ReturnedToReview,
    StartReview,
    FinishReview,
    ReReview,
    ActivateReviewed,
    ParkActive,
    StaleTicket,
    IntegrateReady,
    IntegrationBlocked,
    RemergeNeeded,
    RevertOverlay,
    DeployFailed,
    DeployWaiting,
    ComposeDeployComment,
    PostDeployComment,
    TransitionAlpha,
    ReturnedMention,
    ApprovePrs,
    AdapterUnavailable,
}

impl RuleId {
    pub const ALL: &'static [RuleId] = &[
        RuleId::SyncStale,
        RuleId::ClaimNew,
        RuleId::ReturnedToReview,
        RuleId::StartReview,
        RuleId::FinishReview,
        RuleId::ReReview,
        RuleId::ActivateReviewed,
        RuleId::ParkActive,
        RuleId::StaleTicket,
        RuleId::IntegrateReady,
        RuleId::IntegrationBlocked,
        RuleId::RemergeNeeded,
        RuleId::RevertOverlay,
        RuleId::DeployFailed,
        RuleId::DeployWaiting,
        RuleId::ComposeDeployComment,
        RuleId::PostDeployComment,
        RuleId::TransitionAlpha,
        RuleId::ReturnedMention,
        RuleId::ApprovePrs,
        RuleId::AdapterUnavailable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RuleId::SyncStale => "sync_stale",
            RuleId::ClaimNew => "claim_new",
            RuleId::ReturnedToReview => "returned_to_review",
            RuleId::StartReview => "start_review",
            RuleId::FinishReview => "finish_review",
            RuleId::ReReview => "re_review",
            RuleId::ActivateReviewed => "activate_reviewed",
            RuleId::ParkActive => "park_active",
            RuleId::StaleTicket => "stale_ticket",
            RuleId::IntegrateReady => "integrate_ready",
            RuleId::IntegrationBlocked => "integration_blocked",
            RuleId::RemergeNeeded => "remerge_needed",
            RuleId::RevertOverlay => "revert_overlay",
            RuleId::DeployFailed => "deploy_failed",
            RuleId::DeployWaiting => "deploy_waiting",
            RuleId::ComposeDeployComment => "compose_deploy_comment",
            RuleId::PostDeployComment => "post_deploy_comment",
            RuleId::TransitionAlpha => "transition_alpha",
            RuleId::ReturnedMention => "returned_mention",
            RuleId::ApprovePrs => "approve_prs",
            RuleId::AdapterUnavailable => "adapter_unavailable",
        }
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a suggestion proposes, as a typed value: never a string command. The UI builds
/// nothing itself; it hands the action to the executor ([`exec`](super::exec) or the CLI).
///
/// The only external writes are inside [`SuggestedAction::Gateway`], and executing one is
/// always `Gateway::draft_because(action, suggestion.facts)`, then a human confirmation,
/// then `Draft::confirm`, then `Gateway::execute`. The engine itself never confirms or
/// executes anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "params", rename_all = "snake_case")]
pub enum SuggestedAction {
    /// Refresh the cache from Jira and Bitbucket (read-only).
    Sync,
    /// Start tracking the ticket (or, for a ticket that came back to Review, track it anew).
    Claim { ticket: TicketKey },
    /// Begin (or restart) reviewing the ticket's PRs.
    StartReview { ticket: TicketKey },
    /// Record that the review is finished, covering the branch tips as they are now.
    MarkReviewed { ticket: TicketKey },
    /// Activate for local test. `baseline` is only ever set for hotfixes (chosen first).
    Activate {
        ticket: TicketKey,
        baseline: Option<BaselineChoice>,
    },
    /// A hotfix is about to be activated: ask which branch the untouched repos use.
    ChooseHotfixBaseline { ticket: TicketKey },
    /// Set the active ticket aside (stashing whatever is uncommitted).
    Park { ticket: TicketKey },
    /// Finish with a ticket that is no longer needed.
    MarkDone { ticket: TicketKey },
    /// Build the merge into `uat` in a temporary worktree and report. The push itself is
    /// produced from that result (a `Gateway(PushUat)` action), never by a rule.
    RunIntegrationPrepare { ticket: TicketKey },
    /// Compose the deploy comment as a local draft.
    ComposeDeployComment { ticket: TicketKey, partial: bool },
    /// Undo the test overlay in these repos.
    RevertOverlay {
        ticket: TicketKey,
        repos: Vec<String>,
    },
    /// Informational: a repo cannot be integrated into `uat` as it is.
    ResolveConflict {
        ticket: TicketKey,
        repo: String,
        files: Vec<String>,
    },
    /// Informational: look at a PR.
    OpenPr {
        repo: String,
        pr: u64,
        url: String,
    },
    /// Informational: look at a pipeline run (its step log).
    OpenPipeline {
        repo: String,
        run_id: String,
        url: String,
    },
    /// Informational: look at a ticket (and the comment that mentions you).
    OpenTicket {
        ticket: TicketKey,
        url: Option<String>,
    },
    /// Informational: nothing to do until something finishes.
    Waiting { ticket: TicketKey, what: String },
    /// Informational: a step needs a writer that is not available. Replaces the gateway
    /// suggestion instead of hiding it.
    AdapterUnavailable {
        adapter: String,
        detail: String,
        blocked: String,
    },
    /// An external write. Only ever executed through the gateway.
    Gateway(Action),
}

impl SuggestedAction {
    pub fn level(&self) -> ExecutionLevel {
        match self {
            SuggestedAction::Sync
            | SuggestedAction::Waiting { .. }
            | SuggestedAction::AdapterUnavailable { .. } => ExecutionLevel::Automatic,
            SuggestedAction::Gateway(_) => ExecutionLevel::ConfirmedExternal,
            _ => ExecutionLevel::LocalOneClick,
        }
    }

    /// Executing it changes nothing: it shows something or says what is being waited for.
    pub fn is_informational(&self) -> bool {
        matches!(
            self,
            SuggestedAction::ResolveConflict { .. }
                | SuggestedAction::OpenPr { .. }
                | SuggestedAction::OpenPipeline { .. }
                | SuggestedAction::OpenTicket { .. }
                | SuggestedAction::Waiting { .. }
                | SuggestedAction::AdapterUnavailable { .. }
        )
    }

    /// The snake_case name of the variant, as in the JSON `type` field.
    pub fn kind(&self) -> &'static str {
        match self {
            SuggestedAction::Sync => "sync",
            SuggestedAction::Claim { .. } => "claim",
            SuggestedAction::StartReview { .. } => "start_review",
            SuggestedAction::MarkReviewed { .. } => "mark_reviewed",
            SuggestedAction::Activate { .. } => "activate",
            SuggestedAction::ChooseHotfixBaseline { .. } => "choose_hotfix_baseline",
            SuggestedAction::Park { .. } => "park",
            SuggestedAction::MarkDone { .. } => "mark_done",
            SuggestedAction::RunIntegrationPrepare { .. } => "run_integration_prepare",
            SuggestedAction::ComposeDeployComment { .. } => "compose_deploy_comment",
            SuggestedAction::RevertOverlay { .. } => "revert_overlay",
            SuggestedAction::ResolveConflict { .. } => "resolve_conflict",
            SuggestedAction::OpenPr { .. } => "open_pr",
            SuggestedAction::OpenPipeline { .. } => "open_pipeline",
            SuggestedAction::OpenTicket { .. } => "open_ticket",
            SuggestedAction::Waiting { .. } => "waiting",
            SuggestedAction::AdapterUnavailable { .. } => "adapter_unavailable",
            SuggestedAction::Gateway(_) => "gateway",
        }
    }
}

/// How urgent a suggestion is: bigger is more urgent.
///
/// # The scheme
///
/// Suggestions fall in bands, and a rule picks a number inside its band (the constants in
/// [`prio`]). Order is total and stable: priority descending, then the ticket's manual order
/// (tickets you do not track yet come after tracked ones, by Jira priority, see
/// `engine::sort`), then the ticket key, then the rule, then the subject.
///
/// | Band | Range | What |
/// |---|---|---|
/// | Automatic | 1200 | sync (runs on its own; the data below is only as good as it) |
/// | Blocking | 1000..=1099 | failed deploy, `uat` conflict, an overlay in the way |
/// | Review | 800..=899 | review requests, returned and you are mentioned, re-review |
/// | In flight | 600..=699 | your own tickets' next steps |
/// | Queue | 430..=480 | claim next: Jira priority, then manual order |
/// | Housekeeping | 200..=299 | stale tickets, adapters that are not available |
///
/// A **hotfix** ticket's suggestions are raised by [`prio::HOTFIX_BOOST`] (200), except
/// housekeeping and automatic ones, so a hotfix review request ties with a blocking item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Priority(pub u32);

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The numbers of the priority scheme (see [`Priority`]).
pub mod prio {
    pub const AUTOMATIC: u32 = 1200;

    pub const DEPLOY_FAILED: u32 = 1000;
    pub const DEPLOY_RERUN: u32 = 995;
    pub const INTEGRATION_BLOCKED: u32 = 990;
    pub const REVERT_OVERLAY: u32 = 985;

    pub const RETURNED_TO_REVIEW: u32 = 850;
    pub const RETURNED_MENTION: u32 = 840;
    pub const RE_REVIEW: u32 = 830;
    pub const START_REVIEW: u32 = 820;
    pub const FINISH_REVIEW: u32 = 810;

    pub const APPROVE_PRS: u32 = 690;
    pub const INTEGRATE_READY: u32 = 680;
    pub const COMPOSE_DEPLOY_COMMENT: u32 = 670;
    pub const POST_DEPLOY_COMMENT: u32 = 665;
    pub const TRANSITION_ALPHA: u32 = 660;
    pub const REMERGE_NEEDED: u32 = 645;
    pub const ACTIVATE_REVIEWED: u32 = 640;
    pub const ACTIVATE_PARKED: u32 = 630;
    pub const PARK_ACTIVE_DIRTY: u32 = 625;
    pub const PARK_ACTIVE: u32 = 620;
    pub const DEPLOY_WAITING: u32 = 600;

    /// Claiming: `CLAIM_TOP - CLAIM_STEP * jira_priority_rank` (rank 0 is the most urgent).
    pub const CLAIM_TOP: u32 = 480;
    pub const CLAIM_STEP: u32 = 10;

    pub const STALE_TICKET: u32 = 250;
    /// Ceiling for "adapter not available" notes.
    pub const ADAPTER_UNAVAILABLE: u32 = 299;

    /// Added to a hotfix ticket's suggestions that are at least queue-level.
    pub const HOTFIX_BOOST: u32 = 200;
    /// Below this a suggestion is housekeeping and gets no hotfix boost.
    pub const QUEUE_FLOOR: u32 = 400;
}

/// One thing the author could do now, with the reason and the facts behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    /// Stable across recomputations (derived from ticket, rule and subject), so a dismissal
    /// sticks. See [`Suggestion::make_id`].
    pub id: String,
    pub ticket: Option<TicketKey>,
    pub rule: RuleId,
    pub action: SuggestedAction,
    /// Human-readable, states the facts it rests on.
    pub reason: String,
    /// The same facts, as data. Also what a gateway draft records in the audit log. Must not
    /// contain anything that changes without the situation changing (no "age", no `now`).
    pub facts: Value,
    pub priority: Priority,
    pub level: ExecutionLevel,
    /// Executing it changes nothing (see [`SuggestedAction::is_informational`]).
    pub informational: bool,
}

impl Suggestion {
    /// `<ticket or ->:<rule>:<subject>`. The subject tells apart several suggestions of one
    /// rule for one ticket (a repo, a PR); use `-` when there is only one.
    pub fn make_id(ticket: Option<&TicketKey>, rule: RuleId, subject: &str) -> String {
        format!(
            "{}:{}:{}",
            ticket.map_or("-", TicketKey::as_str),
            rule.as_str(),
            if subject.is_empty() { "-" } else { subject }
        )
    }

    pub fn new(
        ticket: Option<&TicketKey>,
        rule: RuleId,
        subject: &str,
        action: SuggestedAction,
        reason: impl Into<String>,
        facts: Value,
        priority: u32,
    ) -> Self {
        Self {
            id: Self::make_id(ticket, rule, subject),
            ticket: ticket.cloned(),
            rule,
            level: action.level(),
            informational: action.is_informational(),
            action,
            reason: reason.into(),
            facts,
            priority: Priority(priority),
        }
    }

    /// The hash of the facts, which decides whether a response still applies.
    pub fn facts_hash(&self) -> String {
        facts_hash(&self.facts)
    }
}

fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(v, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// A hash of `facts` that is stable across runs, releases and key order (FNV-1a over the
/// key-sorted JSON), as 16 hex digits. It is stored, so it must never change meaning.
pub fn facts_hash(facts: &Value) -> String {
    let mut text = String::new();
    canonical(facts, &mut text);
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
