//! Mirror of Jira comments with the accounts they mention (`cache.db`).

use eyre::Context;
use rusqlite::params;

use super::Store;
use crate::domain::TicketKey;
use crate::providers::RemoteComment;

fn load(
    cache: &Store,
    sql: &str,
    params: impl rusqlite::Params,
) -> eyre::Result<Vec<RemoteComment>> {
    let mut stmt = cache.conn().prepare(sql)?;
    let mut comments = stmt
        .query_map(params, |r| {
            Ok(RemoteComment {
                ticket: r.get(0)?,
                id: r.get(1)?,
                author_account_id: r.get(2)?,
                author_name: r.get(3)?,
                body_text: r.get(4)?,
                rich: r.get(6)?,
                mentions: Vec::new(),
                created_at: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to read cached Jira comments")?;

    let mut mentions = cache.conn().prepare(
        "SELECT account_id FROM jira_comment_mentions
         WHERE ticket_key = ?1 AND comment_id = ?2 ORDER BY account_id",
    )?;
    for c in &mut comments {
        c.mentions = mentions
            .query_map(params![c.ticket, c.id], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
    }
    Ok(comments)
}

const SELECT: &str = "SELECT ticket_key, id, author_account_id, author_name, body_text, created_at, rich
                      FROM jira_comments";

/// Makes the cached comments of `ticket` exactly `comments` (one transaction). Call this
/// only with a complete, successfully fetched list: comments missing from it are removed.
pub fn replace_for_ticket(
    cache: &Store,
    ticket: &TicketKey,
    comments: &[RemoteComment],
    now: i64,
) -> eyre::Result<()> {
    let tx = cache.conn().unchecked_transaction()?;
    // Mentions go with their comment through the foreign key.
    tx.execute(
        "DELETE FROM jira_comments WHERE ticket_key = ?1",
        params![ticket],
    )?;
    for c in comments {
        tx.execute(
            "INSERT OR REPLACE INTO jira_comments
                (ticket_key, id, author_account_id, author_name, body_text, created_at, fetched_at, rich)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ticket,
                c.id,
                c.author_account_id,
                c.author_name,
                c.body_text,
                c.created_at,
                now,
                c.rich
            ],
        )?;
        for account in &c.mentions {
            tx.execute(
                "INSERT OR IGNORE INTO jira_comment_mentions (ticket_key, comment_id, account_id)
                 VALUES (?1, ?2, ?3)",
                params![ticket, c.id, account],
            )?;
        }
    }
    tx.commit()
        .wrap_err_with(|| format!("Failed to cache the comments of {ticket}"))
}

/// The cached comments of a ticket, oldest first.
pub fn list_for_ticket(cache: &Store, ticket: &TicketKey) -> eyre::Result<Vec<RemoteComment>> {
    load(
        cache,
        &format!("{SELECT} WHERE ticket_key = ?1 ORDER BY created_at, id"),
        params![ticket],
    )
}

/// Every cached comment that mentions `account_id`, newest first: the "you were tagged"
/// query. Comments the account wrote itself are included only if they mention it.
pub fn mentioning(cache: &Store, account_id: &str) -> eyre::Result<Vec<RemoteComment>> {
    load(
        cache,
        &format!(
            "{SELECT} WHERE EXISTS (
                SELECT 1 FROM jira_comment_mentions m
                WHERE m.ticket_key = jira_comments.ticket_key
                  AND m.comment_id = jira_comments.id AND m.account_id = ?1)
             ORDER BY created_at DESC, ticket_key, id"
        ),
        params![account_id],
    )
}

/// Comments of one ticket that mention `account_id`, newest first.
pub fn mentioning_in_ticket(
    cache: &Store,
    ticket: &TicketKey,
    account_id: &str,
) -> eyre::Result<Vec<RemoteComment>> {
    Ok(mentioning(cache, account_id)?
        .into_iter()
        .filter(|c| &c.ticket == ticket)
        .collect())
}

/// Removes every cached comment of tickets not in `keep` (used with the ticket cache
/// clean-up after a successful full sync).
pub fn delete_except(cache: &Store, keep: &[TicketKey]) -> eyre::Result<usize> {
    let mut stmt = cache
        .conn()
        .prepare("SELECT DISTINCT ticket_key FROM jira_comments")?;
    let present: Vec<TicketKey> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut removed = 0;
    for key in present.iter().filter(|k| !keep.contains(k)) {
        removed += cache.conn().execute(
            "DELETE FROM jira_comments WHERE ticket_key = ?1",
            params![key],
        )?;
    }
    Ok(removed)
}

#[cfg(test)]
mod rich_tests {
    use super::*;
    use crate::providers::fake::build::comment;
    use crate::store::Kind;

    #[test]
    fn the_rich_body_is_stored_beside_the_plain_text() {
        let cache = Store::open_in_memory(Kind::Cache).unwrap();
        let key: TicketKey = "PROJ-1".parse().unwrap();
        let mut c = comment("PROJ-1", "c1", &["me"]);
        c.body_text = "Plan\nstep".into();
        c.rich = "## Plan\n\n- step".into();
        replace_for_ticket(&cache, &key, &[c], 1).unwrap();

        let back = list_for_ticket(&cache, &key).unwrap();
        assert_eq!(back[0].body_text, "Plan\nstep");
        assert_eq!(back[0].rich, "## Plan\n\n- step");
        assert_eq!(mentioning(&cache, "me").unwrap()[0].rich, "## Plan\n\n- step");
    }
}
