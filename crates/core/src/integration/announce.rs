//! The deploy comment (drafted locally, edited, posted once) and the Alpha Testing
//! transition (offered only after the comment is posted). Both go through the gateway.

use eyre::{bail, eyre};
use serde_json::{Value, json};

use super::{
    actions,
    deploy::{DeployState, RepoDeploy, deploy_status},
};
use crate::{
    config::Config,
    domain::{AuditOutcome, TicketKey},
    gateway::{Action, Confirmed, Draft, Gateway, Outcome},
    providers::{PrState, RemoteComment},
    store::{
        Store,
        audit::{self, NewAuditEntry},
        drafts::{self, DraftKind, DraftStatus, StoredDraft},
        prs,
    },
    sync::HostedRepo,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct ComposeOptions<'a> {
    /// Allow a comment while some repos are not deployed; the body says which.
    pub partial: bool,
    /// Free text appended to the body.
    pub note: Option<&'a str>,
}

fn environment_label(repo: &RepoDeploy) -> String {
    repo.run
        .as_ref()
        .and_then(|run| {
            run.steps
                .iter()
                .find(|s| {
                    s.state == crate::providers::PipelineState::Succeeded
                        && s.deployment_environment.is_some()
                })
                .and_then(|s| s.deployment_environment.clone())
        })
        .or_else(|| repo.environment.clone())
        .unwrap_or_else(|| "the deployment environment".into())
}

/// Composes the deploy comment from the deploy status and the cached PRs and stores it as a
/// draft (replacing an earlier unposted one). Refuses unless every touched repo is deployed,
/// or `partial` is set and at least one is.
pub fn compose_deploy_comment(
    state: &Store,
    cache: &Store,
    ticket: &TicketKey,
    hosted: &[HostedRepo],
    opts: ComposeOptions<'_>,
    now: i64,
) -> eyre::Result<StoredDraft> {
    let status = deploy_status(state, cache, ticket, hosted)?;
    if status.is_empty() {
        bail!("{ticket} has no repos to announce");
    }
    let pending: Vec<&RepoDeploy> = status
        .iter()
        .filter(|r| r.state != DeployState::Deployed)
        .collect();
    if !pending.is_empty() && !opts.partial {
        bail!(
            "Not every repo is deployed ({}); wait, or draft a partial comment with --partial",
            pending
                .iter()
                .map(|r| format!("{}: {}", r.repo, r.state.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if pending.len() == status.len() {
        bail!("Nothing is deployed yet for {ticket}; there is nothing to announce");
    }

    let open_prs = prs::for_ticket(cache, ticket)?;
    let mut lines = Vec::new();
    if pending.is_empty() {
        let mut envs: Vec<String> = status.iter().map(environment_label).collect();
        envs.sort();
        envs.dedup();
        lines.push(format!("{ticket} is deployed to {}.", envs.join(", ")));
    } else {
        lines.push(format!(
            "{ticket} is PARTIALLY deployed. Pending: {}.",
            pending
                .iter()
                .map(|r| format!("{} ({})", r.repo, r.state.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for repo in &status {
        lines.push(String::new());
        lines.push(match &repo.hosting_repo {
            Some(h) => format!("{} ({h})", repo.repo),
            None => repo.repo.clone(),
        });
        if let Some(host) = &repo.hosting_repo {
            for pr in open_prs
                .iter()
                .filter(|p| &p.repo == host && matches!(p.state, PrState::Open | PrState::Merged))
            {
                lines.push(format!("  PR #{}: {}", pr.id, pr.url));
            }
        }
        match (&repo.state, &repo.run) {
            (DeployState::Deployed, Some(run)) => lines.push(format!(
                "  Pipeline {}: {}  (environment: {}, commit {})",
                run.number
                    .map_or_else(|| run.id.clone(), |n| format!("#{n}")),
                run.url,
                environment_label(repo),
                crate::git::short_sha(&run.commit)
            )),
            (s, _) => lines.push(format!("  Not deployed yet ({}).", s.as_str())),
        }
    }
    if let Some(note) = opts.note.map(str::trim).filter(|n| !n.is_empty()) {
        lines.push(String::new());
        lines.push(note.into());
    }

    let draft = drafts::create(
        state,
        ticket,
        DraftKind::DeployComment,
        &lines.join("\n"),
        now,
    )?;
    audit::append(
        state,
        &NewAuditEntry {
            at: now,
            action: actions::DEPLOY_COMMENT_DRAFTED.into(),
            ticket: Some(ticket.clone()),
            repo: None,
            details: json!({ "draft_id": draft.id, "partial": !pending.is_empty() }),
            outcome: AuditOutcome::Success,
        },
    )?;
    Ok(draft)
}

/// Edits the body of an unposted draft.
pub fn update_draft_body(state: &Store, id: i64, body: &str) -> eyre::Result<()> {
    drafts::update_body(state, id, body)
}

pub fn discard_draft(state: &Store, id: i64) -> eyre::Result<()> {
    drafts::discard(state, id)
}

fn draft_id_of(confirmed: &Confirmed) -> Option<i64> {
    // The draft id travels in the confirmed facts, set by the preview functions below.
    let facts: Value = confirmed.facts_json();
    facts.get("draft_id").and_then(Value::as_i64)
}

/// A success entry for this draft in the audit log means it was already sent, whatever the
/// drafts table says.
fn audited_as_sent(state: &Store, ticket: &TicketKey, action: &str, id: i64) -> eyre::Result<bool> {
    Ok(audit::list(state, Some(ticket), 10_000)?.iter().any(|e| {
        e.action == action
            && e.outcome == AuditOutcome::Success
            && e.details
                .pointer("/gateway/facts/draft_id")
                .and_then(Value::as_i64)
                == Some(id)
    }))
}

fn open_draft(state: &Store, id: i64, kind: DraftKind) -> eyre::Result<StoredDraft> {
    let draft = drafts::get(state, id)?.ok_or_else(|| eyre!("There is no draft {id}"))?;
    if draft.kind != kind {
        bail!("Draft {id} is not a {}", kind.as_str());
    }
    match draft.status {
        DraftStatus::Draft => Ok(draft),
        DraftStatus::Posted => bail!("Draft {id} was already posted; it cannot be posted again"),
        DraftStatus::Discarded => bail!("Draft {id} was discarded"),
    }
}

/// The gateway preview of posting the deploy comment `id`, exactly as it will be sent.
pub fn preview_post_comment(gateway: &Gateway<'_>, state: &Store, id: i64) -> eyre::Result<Draft> {
    let draft = open_draft(state, id, DraftKind::DeployComment)?;
    if audited_as_sent(
        state,
        &draft.ticket,
        crate::gateway::actions::JIRA_COMMENT,
        id,
    )? {
        bail!("The audit log shows draft {id} was already posted; not posting it again");
    }
    let action = Action::PostJiraComment {
        ticket: draft.ticket.clone(),
        body: draft.body.clone(),
    };
    gateway.ensure_available(&action)?;
    Ok(gateway.draft_because(action, json!({ "draft_id": id, "kind": "deploy_comment" })))
}

/// Posts the confirmed comment and marks the draft `Posted`. The body must still be the
/// one that was previewed.
pub fn post_comment(
    gateway: &Gateway<'_>,
    state: &Store,
    id: i64,
    confirmed: Confirmed,
    now: i64,
) -> eyre::Result<RemoteComment> {
    let draft = open_draft(state, id, DraftKind::DeployComment)?;
    match confirmed.action() {
        Action::PostJiraComment { ticket, body }
            if ticket == &draft.ticket
                && body == &draft.body
                && draft_id_of(&confirmed) == Some(id) => {}
        _ => bail!("The confirmation is not for the current text of draft {id}; preview it again"),
    }
    if audited_as_sent(
        state,
        &draft.ticket,
        crate::gateway::actions::JIRA_COMMENT,
        id,
    )? {
        bail!("The audit log shows draft {id} was already posted; not posting it again");
    }

    let executed = gateway.execute(confirmed, now)?;
    let Outcome::JiraComment(comment) = executed.outcome else {
        bail!("unexpected outcome posting the comment");
    };
    drafts::mark_posted(state, id, now, Some(&comment.id)).map_err(|e| {
        e.wrap_err(format!(
            "The comment WAS posted (Jira id {}) but draft {id} could not be marked posted",
            comment.id
        ))
    })?;
    let _ = audit::append(
        state,
        &NewAuditEntry {
            at: now,
            action: actions::DEPLOY_COMMENT_POSTED.into(),
            ticket: Some(draft.ticket.clone()),
            repo: None,
            details: json!({ "draft_id": id, "remote_id": comment.id }),
            outcome: AuditOutcome::Success,
        },
    );
    Ok(comment)
}

/// Offers the transition to the configured alpha-testing status. Only after a deploy comment
/// was posted, and only once per posted comment. Returns the stored transition draft id and
/// the gateway preview.
pub fn preview_transition(
    gateway: &Gateway<'_>,
    state: &Store,
    config: &Config,
    ticket: &TicketKey,
    now: i64,
) -> eyre::Result<(i64, Draft)> {
    let comment = drafts::latest(state, ticket, DraftKind::DeployComment, DraftStatus::Posted)?
        .ok_or_else(|| {
            eyre!("Post the deploy comment for {ticket} first; the transition is offered after it")
        })?;
    let done = drafts::latest(state, ticket, DraftKind::Transition, DraftStatus::Posted)?;
    if done.is_some_and(|t| t.id > comment.id) {
        bail!("{ticket} was already transitioned after its latest deploy comment");
    }

    let status = config.jira_statuses().alpha_testing_name().to_string();
    let action = Action::TransitionJira {
        ticket: ticket.clone(),
        to_status: status.clone(),
    };
    gateway.ensure_available(&action)?;
    let stored = drafts::create(state, ticket, DraftKind::Transition, &status, now)?;
    let draft = gateway.draft_because(
        action,
        json!({ "draft_id": stored.id, "kind": "transition", "after_comment": comment.id }),
    );
    Ok((stored.id, draft))
}

/// Executes the confirmed transition and marks its draft `Posted`.
pub fn post_transition(
    gateway: &Gateway<'_>,
    state: &Store,
    id: i64,
    confirmed: Confirmed,
    now: i64,
) -> eyre::Result<()> {
    let draft = open_draft(state, id, DraftKind::Transition)?;
    match confirmed.action() {
        Action::TransitionJira { ticket, to_status }
            if ticket == &draft.ticket
                && to_status == &draft.body
                && draft_id_of(&confirmed) == Some(id) => {}
        _ => bail!("The confirmation is not for transition draft {id}; preview it again"),
    }
    if drafts::latest(
        state,
        &draft.ticket,
        DraftKind::DeployComment,
        DraftStatus::Posted,
    )?
    .is_none()
    {
        bail!(
            "The deploy comment of {} is not posted; not transitioning",
            draft.ticket
        );
    }
    if audited_as_sent(
        state,
        &draft.ticket,
        crate::gateway::actions::JIRA_TRANSITION,
        id,
    )? {
        bail!("The audit log shows this transition was already made");
    }
    gateway.execute(confirmed, now)?;
    drafts::mark_posted(state, id, now, None).map_err(|e| {
        e.wrap_err(format!(
            "The transition WAS made but draft {id} could not be marked posted"
        ))
    })?;
    let _ = audit::append(
        state,
        &NewAuditEntry {
            at: now,
            action: actions::DEPLOY_TRANSITION_POSTED.into(),
            ticket: Some(draft.ticket.clone()),
            repo: None,
            details: json!({ "draft_id": id, "to": draft.body }),
            outcome: AuditOutcome::Success,
        },
    );
    Ok(())
}
