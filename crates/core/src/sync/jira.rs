//! Jira sync: the Review pool, tracked tickets, and comments.

use std::collections::BTreeSet;

use super::SyncContext;
use super::report::{Collector, Problem, SourceReport, Step, SyncSource, local, run_source};
use crate::domain::TicketKey;
use crate::providers::{RemoteTicket, TicketProvider};
use crate::store::{jira_cache, jira_comments, sync_state, tickets};

/// Refreshes the Jira mirror. Reads only (`jira` is a read trait object).
///
/// 1. The **Review pool**: `search(review_jql)`.
/// 2. **Returned** tickets: `search(returned_jql)`, or `status = "<returned>"`.
/// 3. Every **locally tracked** ticket not returned by those, by key (`get`), so a ticket
///    that left the pool is still refreshed.
/// 4. **Comments** (with mentions) of tracked tickets and of Returned tickets.
/// 5. Only if both searches succeeded: cached tickets that are neither in a result nor
///    tracked, and comments of tickets that are neither tracked nor Returned, are removed.
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

    // 4. Comments of tracked and Returned tickets.
    let with_comments: BTreeSet<TicketKey> = tracked
        .iter()
        .cloned()
        .chain(returned.iter().cloned())
        .collect();
    for key in &with_comments {
        match jira.comments(key) {
            Ok(list) => {
                jira_comments::replace_for_ticket(ctx.cache, key, &list, ctx.now).map_err(local)?;
                c.counts.ticket_comments += list.len();
            }
            Err(e) => c.handle(format!("comments of {key}"), &e)?,
        }
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
    }
    Ok(())
}
