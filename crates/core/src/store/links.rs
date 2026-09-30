//! Ticket to repo links (`state.db`): auto-discovered, manual and excluded.

use eyre::Context;
use rusqlite::params;

use super::Store;
use crate::domain::{RepoLinkOrigin, TicketKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoLink {
    pub ticket: TicketKey,
    pub repo: String,
    pub branch: Option<String>,
    pub origin: RepoLinkOrigin,
}

/// A repo found by matching the ticket key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredLink {
    pub repo: String,
    pub branch: Option<String>,
}

/// All links of `ticket` (including excluded ones), ordered by repo name.
pub fn list(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<RepoLink>> {
    let mut stmt = store.conn().prepare(
        "SELECT ticket_key, repo, branch, origin FROM ticket_repos WHERE ticket_key = ?1 ORDER BY repo",
    )?;
    let links = stmt
        .query_map(params![ticket], |r| {
            Ok(RepoLink {
                ticket: r.get(0)?,
                repo: r.get(1)?,
                branch: r.get(2)?,
                origin: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err_with(|| format!("Failed to list repo links of {ticket}"))?;
    Ok(links)
}

/// The links that count: everything except excluded repos.
pub fn list_included(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<RepoLink>> {
    Ok(list(store, ticket)?
        .into_iter()
        .filter(|l| l.origin != RepoLinkOrigin::Excluded)
        .collect())
}

/// Applies a discovery result: `discovered` becomes the set of `Auto` links.
///
/// - new repos are added as `Auto`; known `Auto` links get their branch refreshed;
/// - `Auto` links no longer discovered are removed;
/// - `Manual` and `Excluded` links are never touched, even if discovery reports the repo.
pub fn apply_discovery(
    store: &Store,
    ticket: &TicketKey,
    discovered: &[DiscoveredLink],
) -> eyre::Result<()> {
    let tx = store.conn().unchecked_transaction()?;

    for link in discovered {
        // The WHERE on the conflict branch is what protects manual and excluded rows.
        tx.execute(
            "INSERT INTO ticket_repos (ticket_key, repo, branch, origin) VALUES (?1, ?2, ?3, 'auto')
             ON CONFLICT (ticket_key, repo) DO UPDATE SET branch = excluded.branch
             WHERE ticket_repos.origin = 'auto'",
            params![ticket, link.repo, link.branch],
        )?;
    }

    let keep: Vec<&str> = discovered.iter().map(|l| l.repo.as_str()).collect();
    let stale: Vec<String> = {
        let mut stmt =
            tx.prepare("SELECT repo FROM ticket_repos WHERE ticket_key = ?1 AND origin = 'auto'")?;
        stmt.query_map(params![ticket], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    for repo in stale.iter().filter(|r| !keep.contains(&r.as_str())) {
        tx.execute(
            "DELETE FROM ticket_repos WHERE ticket_key = ?1 AND repo = ?2 AND origin = 'auto'",
            params![ticket, repo],
        )?;
    }

    tx.commit()
        .wrap_err_with(|| format!("Failed to apply repo discovery for {ticket}"))
}

/// Links `repo` by hand (overriding an auto or excluded link of the same repo).
pub fn add_manual(
    store: &Store,
    ticket: &TicketKey,
    repo: &str,
    branch: Option<&str>,
) -> eyre::Result<()> {
    store
        .conn()
        .execute(
            "INSERT INTO ticket_repos (ticket_key, repo, branch, origin) VALUES (?1, ?2, ?3, 'manual')
             ON CONFLICT (ticket_key, repo) DO UPDATE SET branch = excluded.branch, origin = 'manual'",
            params![ticket, repo, branch],
        )
        .wrap_err_with(|| format!("Failed to link {repo} to {ticket}"))?;
    Ok(())
}

/// Excludes `repo` (stale or duplicate branch). Keeps the branch it was linked with so the
/// exclusion can be shown; discovery will not bring the repo back.
pub fn exclude(store: &Store, ticket: &TicketKey, repo: &str) -> eyre::Result<()> {
    store
        .conn()
        .execute(
            "INSERT INTO ticket_repos (ticket_key, repo, branch, origin) VALUES (?1, ?2, NULL, 'excluded')
             ON CONFLICT (ticket_key, repo) DO UPDATE SET origin = 'excluded'",
            params![ticket, repo],
        )
        .wrap_err_with(|| format!("Failed to exclude {repo} from {ticket}"))?;
    Ok(())
}

/// Forgets any link (of any origin) to `repo`; a later discovery may add it again.
pub fn remove(store: &Store, ticket: &TicketKey, repo: &str) -> eyre::Result<bool> {
    let removed = store
        .conn()
        .execute(
            "DELETE FROM ticket_repos WHERE ticket_key = ?1 AND repo = ?2",
            params![ticket, repo],
        )
        .wrap_err_with(|| format!("Failed to remove the link of {repo} to {ticket}"))?;
    Ok(removed > 0)
}
