//! [`Snapshot::load`]: the impure half of the engine. Reads `state.db`, `cache.db` and the
//! repositories (read-only: opening repos, reading refs and status, never a fetch) and
//! packs the result into the plain data the rules see.
//!
//! Nothing here calls a provider or a writer. Whether the gateway's writers are usable is
//! asked with [`Gateway::ensure_available`], which only inspects what the gateway was built
//! with.

use std::collections::{BTreeMap, BTreeSet};

use eyre::Context;

use super::snapshot::{
    Availability, Checklist, DeployInfo, DraftInfo, Mention, MergeInfo, PrInfo, PrepRepo,
    PrepSnapshot, PrepState, ReviewState, Snapshot, SourceState, StatusNames, SyncSnapshot,
    TicketSnapshot, Tracking, Writers, is_testing_complete, status_is,
};
use crate::{
    activation::WorkspaceRepo,
    config::Config,
    domain::{LocalStatus, TicketKey},
    gateway::{Action, Gateway},
    git::GitRepo,
    integration::{self, deploy_status, needs_remerge},
    store::{
        Store,
        audit::{self, AuditEntry},
        drafts::{self, DraftKind, DraftStatus},
        jira_cache::{self, JiraTicket},
        jira_comments, links, notes, overlays, prs, restore, reviews, sync_state, tickets,
        uat_details,
    },
    sync::{HostedRepo, derive_kind},
};

/// Everything [`Snapshot::load`] reads from.
pub struct LoadContext<'a> {
    /// `state.db`.
    pub state: &'a Store,
    /// `cache.db`.
    pub cache: &'a Store,
    pub config: &'a Config,
    /// The workspace's repos (for branch tips, dirty trees, `needs_remerge`).
    pub repos: &'a [WorkspaceRepo],
    /// The repos that have a `[hosting]` section (`HostedRepo::from_workspace_repos`).
    pub hosted: &'a [HostedRepo],
    /// Used only to ask which writers are available.
    pub gateway: &'a Gateway<'a>,
}

fn availability(gateway: &Gateway<'_>, probe: &Action) -> Availability {
    match gateway.ensure_available(probe) {
        Ok(()) => Availability::Ready,
        Err(e) => Availability::Unavailable(e.to_string()),
    }
}

fn writers(gateway: &Gateway<'_>) -> Writers {
    let probe_ticket: TicketKey = "X-1".parse().expect("valid key");
    Writers {
        jira: availability(
            gateway,
            &Action::PostJiraComment {
                ticket: probe_ticket,
                body: String::new(),
            },
        ),
        code_host: availability(
            gateway,
            &Action::ApprovePr {
                repo: String::new(),
                pr: 0,
            },
        ),
    }
}

/// The tips of the ticket's branches as they are now, by workspace project: the
/// remote-tracking branch when there is one (what a PR contains), else the local branch.
/// Only links with a known branch count; unreadable repos are left out.
pub fn ticket_heads(
    state: &Store,
    repos: &[WorkspaceRepo],
    ticket: &TicketKey,
) -> eyre::Result<BTreeMap<String, String>> {
    let mut heads = BTreeMap::new();
    for link in links::list_included(state, ticket)? {
        let (Some(branch), Some(repo)) = (
            link.branch.as_deref(),
            repos.iter().find(|r| r.name == link.repo),
        ) else {
            continue;
        };
        let Ok(git) = GitRepo::open(&repo.dir) else {
            continue;
        };
        let tip = git
            .rev_parse(&format!("refs/remotes/{}/{branch}", repo.remote()))
            .or_else(|_| git.rev_parse(&format!("refs/heads/{branch}")));
        if let Ok(tip) = tip {
            heads.insert(link.repo, tip);
        }
    }
    Ok(heads)
}

/// The result of the latest `integration prepare` of the ticket, while it still describes
/// the branches: dropped once the ticket was finalized since, and per repo once the ticket
/// branch moved on from the tip that was prepared.
fn load_prep(
    entries: &[AuditEntry],
    state: &Store,
    ticket: &TicketKey,
) -> eyre::Result<Option<PrepSnapshot>> {
    // Newest first.
    let Some(prepared) = entries
        .iter()
        .find(|e| e.action == integration::actions::PREPARED)
    else {
        return Ok(None);
    };
    if entries
        .iter()
        .any(|e| e.action == integration::actions::FINALIZED && e.id > prepared.id)
    {
        return Ok(None);
    }
    let branches: BTreeMap<String, (String, std::path::PathBuf)> = restore::list(state, ticket)?
        .into_iter()
        .map(|r| (r.repo, (r.branch, r.repo_dir)))
        .collect();

    let mut out = Vec::new();
    for r in prepared
        .details
        .get("repos")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(repo), Some(name)) = (
            r.get("repo").and_then(|v| v.as_str()),
            r.get("state").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let Some(state_kind) = PrepState::parse(name) else {
            continue;
        };
        // A result about a tip that is no longer the branch's says nothing about it.
        if let (Some(tip), Some((branch, dir))) = (
            r.get("ticket_tip").and_then(|v| v.as_str()),
            branches.get(repo),
        ) {
            let current = GitRepo::open(dir)
                .and_then(|g| g.rev_parse(&format!("refs/heads/{branch}")))
                .ok();
            if current.is_some_and(|c| c != tip) {
                continue;
            }
        }
        out.push(PrepRepo {
            repo: repo.into(),
            state: state_kind,
            files: r
                .get("files")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|f| f.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            reason: r.get("reason").and_then(|v| v.as_str()).map(String::from),
        });
    }
    Ok((!out.is_empty()).then_some(PrepSnapshot {
        at: prepared.at,
        repos: out,
    }))
}

fn pr_infos(cache: &Store, hosted: &[HostedRepo], key: &TicketKey) -> eyre::Result<Vec<PrInfo>> {
    Ok(prs::for_ticket(cache, key)?
        .into_iter()
        .map(|pr| PrInfo {
            project: HostedRepo::find(hosted, &pr.repo).map(|h| h.project.clone()),
            pr,
        })
        .collect())
}

fn base_snapshot(
    ctx: &LoadContext<'_>,
    statuses: &StatusNames,
    me: Option<&str>,
    key: &TicketKey,
    jira: Option<&JiraTicket>,
    tracking: Option<&tickets::TicketTracking>,
) -> eyre::Result<TicketSnapshot> {
    let mut t = TicketSnapshot::new(key.clone());
    if let Some(j) = jira {
        t.title = Some(j.title.clone());
        t.url = j.url.clone();
        t.jira_status = Some(j.jira_status.clone());
        t.jira_priority = j.priority.clone();
        t.jira_fetched_at = Some(j.fetched_at);
    }
    let prs = pr_infos(ctx.cache, ctx.hosted, key)?;
    let raw: Vec<_> = prs.iter().map(|p| p.pr.clone()).collect();
    t.kind = derive_kind(tracking.and_then(|tr| tr.kind_override), &raw, ctx.hosted).kind;
    t.prs = prs;
    t.tracking = tracking.map(|tr| Tracking {
        status: tr.status,
        manual_order: tr.manual_order,
        claimed_at: tr.claimed_at,
        updated_at: tr.updated_at,
    });

    // Comments that mention the author matter on Returned tickets only.
    if let Some(me) = me
        && status_is(t.jira_status.as_deref(), &statuses.returned)
    {
        t.mentions = jira_comments::mentioning_in_ticket(ctx.cache, key, me)?
            .into_iter()
            .map(|c| Mention {
                comment_id: c.id,
                author: c.author_name,
                text: c.body_text,
                created_at: c.created_at,
            })
            .collect();
    }
    Ok(t)
}

fn fill_tracked(
    ctx: &LoadContext<'_>,
    statuses: &StatusNames,
    t: &mut TicketSnapshot,
) -> eyre::Result<()> {
    let state = ctx.state;
    let key = t.key.clone();
    let status = t.status();

    let items = notes::list_checklist(state, &key)?;
    t.checklist = Checklist {
        total: items.len(),
        done: items.iter().filter(|i| i.done).count(),
    };

    let records = restore::list(state, &key)?;
    t.touched_repos = records
        .iter()
        .filter(|r| r.role == restore::RepoRole::Ticket)
        .map(|r| r.repo.clone())
        .collect();
    t.overlays = overlays::list(state, &key)?
        .into_iter()
        .map(|o| o.repo)
        .collect();
    if status == Some(LocalStatus::Active) {
        for r in records
            .iter()
            .filter(|r| r.role == restore::RepoRole::Ticket)
        {
            let dirty = GitRepo::open(&r.repo_dir)
                .and_then(|g| g.status())
                .is_ok_and(|s| !s.is_clean());
            if dirty {
                t.dirty_repos.push(r.repo.clone());
            }
        }
    }

    // Review marker and the tips now.
    if let Some(mark) = reviews::get(state, &key)? {
        t.review = ReviewState {
            reviewed_at: Some(mark.reviewed_at),
            reviewed_heads: mark.heads,
            current_heads: ticket_heads(state, ctx.repos, &key)?,
        };
    }

    // Recorded merges, and whether the branch moved on since.
    let merges = uat_details::list_for_ticket(state, &key)?;
    let repo_names: BTreeSet<&str> = merges.iter().map(|m| m.merge.repo.as_str()).collect();
    for name in repo_names {
        if let Some(latest) = merges.iter().rfind(|m| m.merge.repo == name) {
            t.merges.push(MergeInfo {
                repo: name.into(),
                commit: latest.merge.commit.clone(),
                recorded_at: latest.merge.recorded_at,
            });
        }
        if let Some(repo) = ctx.repos.iter().find(|r| r.name == name)
            && needs_remerge(state, &key, repo).unwrap_or(false)
        {
            t.remerge_repos.push(name.into());
        }
    }
    if !t.merges.is_empty() {
        t.deploy = deploy_status(state, ctx.cache, &key, ctx.hosted)?
            .into_iter()
            .map(|d| DeployInfo {
                repo: d.repo,
                hosting_repo: d.hosting_repo,
                state: d.state,
                run_id: d.run.as_ref().map(|r| r.id.clone()),
                run_number: d.run.as_ref().and_then(|r| r.number),
                run_url: d.run.as_ref().map(|r| r.url.clone()),
            })
            .collect();
    }

    // Drafts.
    t.comment_draft = drafts::latest(state, &key, DraftKind::DeployComment, DraftStatus::Draft)?
        .map(|d| DraftInfo {
            id: d.id,
            body: d.body,
            created_at: d.created_at,
        });
    t.comment_posted_at =
        drafts::latest(state, &key, DraftKind::DeployComment, DraftStatus::Posted)?
            .and_then(|d| d.posted_at);
    t.transition_posted_at =
        drafts::latest(state, &key, DraftKind::Transition, DraftStatus::Posted)?
            .and_then(|d| d.posted_at);

    // The audit trail: the last prepare, and when the ticket was last handled.
    let entries = audit::list(state, Some(&key), 500)?;
    t.prep = load_prep(&entries, state, &key)?;
    let tracking_updated = t.tracking.as_ref().map(|tr| tr.updated_at);
    t.last_handled_at = entries.first().map(|e| e.at).max(tracking_updated);

    let alpha_reached = matches!(status, Some(LocalStatus::Integrated | LocalStatus::Done))
        || !t.merges.is_empty()
        || t.transition_posted_at.is_some();
    t.testing_complete = is_testing_complete(statuses, t.jira_status.as_deref(), alpha_reached);
    Ok(())
}

impl Snapshot {
    /// Gathers everything the rules need. Read-only towards git, Jira and Bitbucket.
    pub fn load(ctx: &LoadContext<'_>) -> eyre::Result<Snapshot> {
        let statuses = StatusNames::from(&ctx.config.jira_statuses());
        let me = ctx.config.jira_account_id().map(String::from);

        let tracked = tickets::list(ctx.state)?;
        let cached = jira_cache::list(ctx.cache)?;

        let mut out = Vec::new();
        for tr in &tracked {
            let jira = cached.iter().find(|j| j.key == tr.key);
            let mut t = base_snapshot(ctx, &statuses, me.as_deref(), &tr.key, jira, Some(tr))?;
            fill_tracked(ctx, &statuses, &mut t)
                .wrap_err_with(|| format!("Failed to read the state of {}", tr.key))?;
            out.push(t);
        }
        for j in cached
            .iter()
            .filter(|j| !tracked.iter().any(|tr| tr.key == j.key))
        {
            out.push(base_snapshot(
                ctx,
                &statuses,
                me.as_deref(),
                &j.key,
                Some(j),
                None,
            )?);
        }

        let mut expected = Vec::new();
        if ctx.config.jira.is_some() {
            expected.push(sync_state::JIRA.to_string());
        }
        expected.extend(
            ctx.hosted
                .iter()
                .map(|h| sync_state::code_host_source(&h.repo)),
        );
        let states = sync_state::list(ctx.cache)?
            .into_iter()
            .map(|s| SourceState {
                source: s.source,
                last_ok_at: s.last_ok_at,
                last_attempt_at: s.last_attempt_at,
                last_error: s.last_error,
            })
            .collect();

        Ok(Snapshot {
            me,
            statuses,
            tickets: out,
            sync: SyncSnapshot { expected, states },
            writers: writers(ctx.gateway),
        })
    }
}
