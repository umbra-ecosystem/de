//! Jira sync: the Review pool, tracked tickets, and comments.

use std::collections::{BTreeMap, BTreeSet};

use super::{FULL_REFRESH_AFTER_SECS, SyncContext};
use super::report::{Collector, Problem, SourceReport, Step, SyncSource, local, run_source};
use crate::domain::TicketKey;
use crate::providers::{RemoteTicket, TicketProvider};
use crate::store::{jira_cache, jira_comments, jira_details, sync_state, tickets};

/// Refreshes the Jira mirror. Reads only (`jira` is a read trait object).
///
/// 1. The **Review pool**: `search(review_jql)`.
/// 2. **Returned** tickets: `search(returned_jql)`, or `status = "<returned>"`.
/// 3. Every **locally tracked** ticket not returned by those, by key (`get`), so a ticket
///    that left the pool is still refreshed.
/// 4. **Comments** (with mentions) and **detail** (description, reporter, sprint, links...) of every
///    ticket found or tracked, in one `view` call each.
/// 5. Only if both searches succeeded: cached tickets that are neither in a result nor
///    tracked, with their comments and detail, are removed.
///
/// An environmental error (offline, not logged in) aborts with the cache untouched from
/// that point on; a per-ticket error (deleted ticket, unparsable comment) is recorded as a
/// problem and the rest continues.
pub fn sync_jira(ctx: &SyncContext<'_>, jira: &dyn TicketProvider) -> SourceReport {
    run_source(ctx, SyncSource::Jira, sync_state::JIRA.into(), |c| {
        jira_body(ctx, jira, c)
    })
}

fn quote_jql(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Runs one search and caches its tickets. `Ok(None)` means the search failed for a
/// non-environmental reason (already recorded), so the result is not complete.
fn search_and_cache(
    ctx: &SyncContext<'_>,
    jira: &dyn TicketProvider,
    c: &mut Collector,
    what: &str,
    jql: &str,
) -> Result<Option<Vec<RemoteTicket>>, Problem> {
    crate::synclog::log(format!("{what}: {jql}"));
    match jira.search(jql) {
        Ok(found) => {
            crate::synclog::log(format!("{what}: {} ticket(s)", found.len()));
            for t in &found {
                jira_cache::upsert_remote(ctx.cache, t, ctx.now).map_err(local)?;
            }
            c.counts.tickets += found.len();
            Ok(Some(found))
        }
        Err(e) => {
            c.handle(what, &e)?;
            Ok(None)
        }
    }
}

/// Which tickets need a full `view`: those never fetched or fetched too long ago, everything on a `full` sync,
/// and of the rest only those Jira reports as updated since they were cached. Tickets cached together are asked
/// about together (by how long ago, in ten-minute steps plus a margin), so one search answers for most of them.
/// When Jira cannot be asked, the tickets are fetched: slower, never stale.
fn plan_views(
    ctx: &SyncContext<'_>,
    jira: &dyn TicketProvider,
    c: &mut Collector,
    wanted: &BTreeSet<TicketKey>,
) -> Result<BTreeSet<TicketKey>, Problem> {
    let fetched = jira_details::fetched_at(ctx.cache).map_err(local)?;
    let mut need = BTreeSet::new();
    let mut groups: BTreeMap<u32, Vec<TicketKey>> = BTreeMap::new();
    for key in wanted {
        match fetched.get(key) {
            Some(at) if !ctx.full && ctx.now - at < FULL_REFRESH_AFTER_SECS => {
                let minutes = u32::try_from((ctx.now - at).max(0) / 60).unwrap_or(u32::MAX);
                let window = (minutes / 10 + 1).saturating_mul(10).saturating_add(5);
                groups.entry(window).or_default().push(key.clone());
            }
            _ => {
                need.insert(key.clone());
            }
        }
    }
    for (window, group) in groups {
        match jira.changed_since(&group, window) {
            Ok(changed) => need.extend(changed),
            Err(e) => {
                c.handle("which tickets changed", &e)?;
                need.extend(group);
            }
        }
    }
    Ok(need)
}

fn jira_body(ctx: &SyncContext<'_>, jira: &dyn TicketProvider, c: &mut Collector) -> Step {
    let jira_config = ctx.config.jira();
    let statuses = ctx.config.jira_statuses();

    let tracked: Vec<TicketKey> = tickets::list(ctx.state)
        .map_err(local)?
        .into_iter()
        .map(|t| t.key)
        .collect();

    let mut seen: BTreeSet<TicketKey> = BTreeSet::new();
    let mut returned: BTreeSet<TicketKey> = BTreeSet::new();
    let mut complete = true;

    // 1. The Review pool.
    match search_and_cache(ctx, jira, c, "Review pool", &ctx.config.review_jql())? {
        Some(found) => seen.extend(found.into_iter().map(|t| t.key)),
        None => complete = false,
    }

    // 2. Returned tickets.
    let returned_jql = jira_config
        .returned_jql
        .clone()
        .unwrap_or_else(|| format!("status = {}", quote_jql(statuses.returned_name())));
    match search_and_cache(ctx, jira, c, "Returned tickets", &returned_jql)? {
        Some(found) => {
            for t in found {
                returned.insert(t.key.clone());
                seen.insert(t.key);
            }
        }
        None => complete = false,
    }

    // 3. Tracked tickets that neither search returned.
    let unseen: Vec<&TicketKey> = tracked.iter().filter(|k| !seen.contains(*k)).collect();
    for key in unseen {
        match jira.get(key) {
            Ok(t) => {
                jira_cache::upsert_remote(ctx.cache, &t, ctx.now).map_err(local)?;
                c.counts.tickets += 1;
                seen.insert(key.clone());
            }
            Err(e) => c.handle(key.to_string(), &e)?,
        }
    }

    // 4. Comments and detail, in full only for what changed since it was cached.
    let with_comments: BTreeSet<TicketKey> = tracked
        .iter()
        .cloned()
        .chain(seen.iter().cloned())
        .collect();
    let to_view = plan_views(ctx, jira, c, &with_comments)?;
    for key in &to_view {
        match jira.view(key) {
            Ok(viewed) => {
                jira_comments::replace_for_ticket(ctx.cache, key, &viewed.comments, ctx.now)
                    .map_err(local)?;
                c.counts.ticket_comments += viewed.comments.len();
                if let Some(detail) = &viewed.detail {
                    jira_details::upsert(ctx.cache, key, detail, ctx.now).map_err(local)?;
                }
            }
            Err(e) => c.handle(format!("comments of {key}"), &e)?,
        }
    }
    if to_view.len() < with_comments.len() {
        c.notes.push(format!(
            "{} of {} tickets changed since they were cached; the rest were kept as they were",
            to_view.len(),
            with_comments.len()
        ));
    }

    c.notes.extend(jira.take_warnings());

    // 5. Stale rows, only when both searches proved what is in the pool.
    if complete {
        let keep_tickets: Vec<TicketKey> = seen
            .iter()
            .cloned()
            .chain(tracked.iter().cloned())
            .collect();
        let keep_comments: Vec<TicketKey> = with_comments.iter().cloned().collect();
        c.counts.removed += jira_cache::delete_except(ctx.cache, &keep_tickets).map_err(local)?;
        c.counts.removed +=
            jira_comments::delete_except(ctx.cache, &keep_comments).map_err(local)?;
        // Details go with their ticket; they are not counted as removed rows of their own.
        jira_details::delete_except(ctx.cache, &keep_tickets).map_err(local)?;
    }
    Ok(())
}
