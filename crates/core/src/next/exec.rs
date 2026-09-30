//! Carrying out a suggestion. Thin glue over the existing flows; the engine itself never
//! calls any of this.
//!
//! - **Local** actions ([`execute_local`]) run the core function that already does the work
//!   (`tickets::claim`, `activation::activate`, `overlay::revert`, ...).
//! - **External** actions are always `Gateway::draft_because(action, suggestion.facts)`, a
//!   human confirmation, `Draft::confirm`, then execute ([`prepare_external`] and
//!   [`PreparedExternal::confirm`]). Nothing here confirms on the caller's behalf.

use eyre::{Context, bail, eyre};
use serde_json::{Value, json};

use super::{
    load::ticket_heads,
    model::{SuggestedAction, Suggestion},
    responses::make_response,
};
use crate::{
    activation::{
        ActivateOptions, ActivationReport, DeactivationReport, WorkspaceRepo, activate,
        discover_links, park,
    },
    config::Config,
    domain::{AuditOutcome, BaselineChoice, LocalStatus, TicketKey},
    gateway::{Action, Confirmed, Draft, Executed, Gateway},
    integration::{
        ComposeOptions, compose_deploy_comment, post_comment, post_transition,
        preview_post_comment, preview_transition,
    },
    overlay::{CommandRunner, RevertOutcome, revert},
    providers::RemoteComment,
    store::{
        Store,
        audit::{self, NewAuditEntry},
        drafts::StoredDraft,
        reviews, suggestion_responses as responses,
        suggestion_responses::ResponseKind,
        tickets,
    },
    sync::HostedRepo,
};

/// Everything the local executors need.
pub struct LocalEnv<'a> {
    pub state: &'a Store,
    pub cache: &'a Store,
    pub repos: &'a [WorkspaceRepo],
    pub hosted: &'a [HostedRepo],
    pub runner: &'a dyn CommandRunner,
    /// The workspace's `default_branch`, for activation.
    pub workspace_default_branch: Option<String>,
    /// Fetch every repo when activating.
    pub fetch: bool,
}

#[derive(Debug)]
pub enum LocalOutcome {
    /// A one-line result (claimed, reviewed, ...).
    Done(String),
    Activated(ActivationReport),
    Parked(DeactivationReport),
    Drafted(StoredDraft),
    Reverted(Vec<(String, RevertOutcome)>),
}

fn audit_local(
    state: &Store,
    now: i64,
    action: &str,
    ticket: &TicketKey,
    details: Value,
) -> eyre::Result<()> {
    audit::append(
        state,
        &NewAuditEntry {
            at: now,
            action: action.into(),
            ticket: Some(ticket.clone()),
            repo: None,
            details,
            outcome: AuditOutcome::Success,
        },
    )?;
    Ok(())
}

fn require(state: &Store, key: &TicketKey) -> eyre::Result<tickets::TicketTracking> {
    tickets::get(state, key)?.ok_or_else(|| eyre!("{key} is not tracked"))
}

/// Claims `key`; a ticket that was integrated (or done) and came back to Review starts over
/// as claimed.
pub fn claim(state: &Store, key: &TicketKey, now: i64) -> eyre::Result<String> {
    match tickets::get(state, key)? {
        None => {
            tickets::claim(state, key, now)?;
            audit_local(state, now, "ticket.claimed", key, json!({ "again": false }))?;
            Ok(format!("Claimed {key}"))
        }
        Some(t) if matches!(t.status, LocalStatus::Integrated | LocalStatus::Done) => {
            if t.status == LocalStatus::Integrated {
                tickets::set_status(state, key, LocalStatus::Done, now)?;
            }
            tickets::set_status(state, key, LocalStatus::Claimed, now)?;
            reviews::clear(state, key)?;
            audit_local(state, now, "ticket.claimed", key, json!({ "again": true }))?;
            Ok(format!("Claimed {key} again (it came back to Review)"))
        }
        Some(t) => bail!("{key} is already tracked ({})", t.status),
    }
}

/// Begins (or restarts) the review: `Claimed`, `Parked` and `Integrated` tickets become
/// `Reviewing`; the reviewed marker is cleared.
pub fn start_review(state: &Store, key: &TicketKey, now: i64) -> eyre::Result<String> {
    let t = require(state, key)?;
    if t.status != LocalStatus::Reviewing && t.status.can_transition_to(LocalStatus::Reviewing) {
        tickets::set_status(state, key, LocalStatus::Reviewing, now)?;
    }
    reviews::clear(state, key)?;
    audit_local(
        state,
        now,
        "review.started",
        key,
        json!({ "from": t.status.as_str() }),
    )?;
    Ok(format!("Review of {key} started"))
}

/// Records the review as finished, covering the ticket branches' tips as they are now.
pub fn mark_reviewed(
    state: &Store,
    repos: &[WorkspaceRepo],
    key: &TicketKey,
    now: i64,
) -> eyre::Result<String> {
    let t = require(state, key)?;
    if t.status == LocalStatus::Claimed {
        tickets::set_status(state, key, LocalStatus::Reviewing, now)?;
    }
    discover_links(state, key, repos, now)?;
    let heads = ticket_heads(state, repos, key)?;
    reviews::mark(state, key, &heads, now)?;
    audit_local(
        state,
        now,
        "review.finished",
        key,
        json!({ "heads": heads }),
    )?;
    Ok(format!("{key} marked as reviewed"))
}

/// Runs a local (non-external) suggestion. Actions that need a person to choose something
/// first (`ChooseHotfixBaseline`), that hand over to another flow (`RunIntegrationPrepare`,
/// `Sync`) or that only inform are refused here; the caller deals with them.
pub fn execute_local(
    env: &LocalEnv<'_>,
    action: &SuggestedAction,
    now: i64,
) -> eyre::Result<LocalOutcome> {
    let state = env.state;
    match action {
        SuggestedAction::Claim { ticket } => claim(state, ticket, now).map(LocalOutcome::Done),
        SuggestedAction::StartReview { ticket } => {
            start_review(state, ticket, now).map(LocalOutcome::Done)
        }
        SuggestedAction::MarkReviewed { ticket } => {
            mark_reviewed(state, env.repos, ticket, now).map(LocalOutcome::Done)
        }
        SuggestedAction::MarkDone { ticket } => {
            tickets::set_status(state, ticket, LocalStatus::Done, now)?;
            audit_local(state, now, "ticket.done", ticket, json!({}))?;
            Ok(LocalOutcome::Done(format!("{ticket} marked done")))
        }
        SuggestedAction::Park { ticket } => {
            Ok(LocalOutcome::Parked(park(state, env.runner, ticket, now)?))
        }
        SuggestedAction::Activate { ticket, baseline } => {
            let options = ActivateOptions {
                fetch: env.fetch,
                baseline: baseline.unwrap_or(BaselineChoice::Base),
                workspace_default_branch: env.workspace_default_branch.clone(),
            };
            Ok(LocalOutcome::Activated(activate(
                state, env.runner, ticket, env.repos, &options, now,
            )?))
        }
        SuggestedAction::RevertOverlay { ticket, repos } => {
            let mut done = Vec::new();
            for repo in repos {
                done.push((repo.clone(), revert(state, env.runner, ticket, repo)?));
            }
            Ok(LocalOutcome::Reverted(done))
        }
        SuggestedAction::ComposeDeployComment { ticket, partial } => {
            Ok(LocalOutcome::Drafted(compose_deploy_comment(
                state,
                env.cache,
                ticket,
                env.hosted,
                ComposeOptions {
                    partial: *partial,
                    note: None,
                },
                now,
            )?))
        }
        other => bail!(
            "'{}' cannot be run by the core executor (it needs a choice, another flow, or only informs)",
            other.kind()
        ),
    }
}

/// What to do after an external action ran, beyond the gateway's own audit.
#[derive(Debug, Clone, Copy)]
enum Follow {
    Nothing,
    PostComment(i64),
    Transition(i64),
}

/// An external suggestion, drafted: show [`preview`](Self::preview), ask, then
/// [`confirm`](Self::confirm).
pub struct PreparedExternal {
    draft: Draft,
    follow: Follow,
}

/// The human said yes. Consumed by [`ConfirmedExternal::execute`].
pub struct ConfirmedExternal {
    confirmed: Confirmed,
    follow: Follow,
}

#[derive(Debug)]
pub enum ExternalOutcome {
    CommentPosted(RemoteComment),
    Transitioned,
    Executed(Executed),
}

impl PreparedExternal {
    pub fn preview(&self) -> &crate::gateway::ActionPreview {
        self.draft.preview()
    }

    pub fn action(&self) -> &Action {
        self.draft.action()
    }

    /// Call only after showing the preview and receiving an explicit yes.
    pub fn confirm(self) -> ConfirmedExternal {
        ConfirmedExternal {
            confirmed: self.draft.confirm(),
            follow: self.follow,
        }
    }
}

/// Drafts the gateway action of `suggestion`: `Gateway::draft_because(action, facts)`.
/// Deploy-comment and transition suggestions first go through the announce flow's own checks
/// (draft still open, not already sent, writer available). `PushUat` is never drafted from a
/// suggestion: it is produced by the integration prepare.
pub fn prepare_external(
    gateway: &Gateway<'_>,
    state: &Store,
    config: &Config,
    suggestion: &Suggestion,
    now: i64,
) -> eyre::Result<PreparedExternal> {
    let SuggestedAction::Gateway(action) = &suggestion.action else {
        bail!("{} is not an external action", suggestion.id);
    };
    let mut facts = suggestion.facts.clone();
    let follow = match action {
        Action::PushUat(_) => {
            bail!("A push to uat comes from `RunIntegrationPrepare`, not from a suggestion")
        }
        Action::PostJiraComment { .. } => {
            let id = facts
                .get("draft_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| eyre!("the suggestion names no draft"))?;
            let checked = preview_post_comment(gateway, state, id)?;
            if checked.action() != action {
                bail!("Draft {id} changed since the suggestion was made; recompute it");
            }
            Follow::PostComment(id)
        }
        Action::TransitionJira { .. } => {
            let ticket = action.ticket().cloned().ok_or_else(|| eyre!("no ticket"))?;
            let (id, checked) = preview_transition(gateway, state, config, &ticket, now)?;
            if checked.action() != action {
                bail!("The alpha status changed since the suggestion was made; recompute it");
            }
            if let Value::Object(map) = &mut facts {
                map.insert("draft_id".into(), json!(id));
                map.insert("kind".into(), json!("transition"));
            }
            Follow::Transition(id)
        }
        _ => {
            gateway.ensure_available(action)?;
            Follow::Nothing
        }
    };
    Ok(PreparedExternal {
        draft: gateway.draft_because(action.clone(), facts),
        follow,
    })
}

impl ConfirmedExternal {
    pub fn execute(
        self,
        gateway: &Gateway<'_>,
        state: &Store,
        now: i64,
    ) -> eyre::Result<ExternalOutcome> {
        match self.follow {
            Follow::PostComment(id) => Ok(ExternalOutcome::CommentPosted(post_comment(
                gateway,
                state,
                id,
                self.confirmed,
                now,
            )?)),
            Follow::Transition(id) => {
                post_transition(gateway, state, id, self.confirmed, now)?;
                Ok(ExternalOutcome::Transitioned)
            }
            Follow::Nothing => Ok(ExternalOutcome::Executed(
                gateway.execute(self.confirmed, now)?,
            )),
        }
    }
}

/// Whether carrying out `action` is remembered as a `Done` response.
///
/// Only for external writes: the remote cache lags behind them, so the suggestion would
/// come back until the next sync. A local action changes the local store at once, which
/// removes the suggestion by itself; remembering it as done would instead hide the same
/// suggestion the next time it legitimately recurs with identical facts (a ticket parked a
/// second time, a review restarted), because the facts of those rules carry no counter.
pub fn records_done(action: &SuggestedAction) -> bool {
    matches!(action, SuggestedAction::Gateway(_))
}

/// Stores the author's response to `suggestion`, tied to its current facts.
pub fn respond(
    state: &Store,
    suggestion: &Suggestion,
    kind: ResponseKind,
    reason: Option<String>,
    snooze_until: Option<i64>,
    now: i64,
) -> eyre::Result<()> {
    responses::record(
        state,
        &make_response(suggestion, kind, reason, snooze_until, now),
    )
    .wrap_err("Failed to store the response")
}
