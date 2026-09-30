//! The detail of cached Jira tickets (`cache.db`): what a search row does not carry.

use std::collections::BTreeMap;

use eyre::Context;
use rusqlite::params;

use super::Store;
use crate::domain::TicketKey;
use crate::providers::TicketDetail;

/// Stores (or replaces) the detail of `key`.
pub fn upsert(cache: &Store, key: &TicketKey, detail: &TicketDetail, now: i64) -> eyre::Result<()> {
    let json = serde_json::to_string(detail).wrap_err("Failed to serialise the ticket detail")?;
    cache
        .conn()
        .execute(
            "INSERT INTO jira_details (key, json, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
            params![key, json, now],
        )
        .wrap_err_with(|| format!("Failed to cache the detail of {key}"))?;
    Ok(())
}

/// Every stored detail. A row that no longer parses (written by another version) is skipped: the ticket then
/// shows without detail until the next sync rewrites it.
pub fn all(cache: &Store) -> eyre::Result<BTreeMap<TicketKey, TicketDetail>> {
    let mut stmt = cache.conn().prepare("SELECT key, json FROM jira_details")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, TicketKey>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to read the cached ticket details")?;
    Ok(rows
        .into_iter()
        .filter_map(|(key, json)| Some((key, serde_json::from_str(&json).ok()?)))
        .collect())
}

pub fn get(cache: &Store, key: &TicketKey) -> eyre::Result<Option<TicketDetail>> {
    Ok(all(cache)?.remove(key))
}

/// Removes the details of every ticket not in `keep`; returns how many went.
pub fn delete_except(cache: &Store, keep: &[TicketKey]) -> eyre::Result<usize> {
    let mut removed = 0;
    for key in all(cache)?.into_keys().filter(|k| !keep.contains(k)) {
        removed += cache
            .conn()
            .execute("DELETE FROM jira_details WHERE key = ?1", params![key])
            .wrap_err_with(|| format!("Failed to remove the cached detail of {key}"))?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Kind;

    fn key(s: &str) -> TicketKey {
        s.parse().unwrap()
    }

    #[test]
    fn stores_replaces_and_prunes() {
        let cache = Store::open_in_memory(Kind::Cache).unwrap();
        let mut d = TicketDetail {
            description: "first".into(),
            labels: vec!["web".into()],
            ..TicketDetail::default()
        };
        upsert(&cache, &key("PROJ-1"), &d, 1).unwrap();
        upsert(&cache, &key("PROJ-2"), &TicketDetail::default(), 1).unwrap();

        d.description = "second".into();
        upsert(&cache, &key("PROJ-1"), &d, 2).unwrap();
        assert_eq!(get(&cache, &key("PROJ-1")).unwrap().unwrap().description, "second");

        assert_eq!(delete_except(&cache, &[key("PROJ-1")]).unwrap(), 1);
        assert!(get(&cache, &key("PROJ-2")).unwrap().is_none());
        assert_eq!(all(&cache).unwrap().len(), 1);
    }

    #[test]
    fn an_unreadable_row_is_skipped_not_fatal() {
        let cache = Store::open_in_memory(Kind::Cache).unwrap();
        cache
            .conn()
            .execute(
                "INSERT INTO jira_details (key, json, fetched_at) VALUES ('PROJ-9', 'not json', 1)",
                [],
            )
            .unwrap();
        assert!(all(&cache).unwrap().is_empty());
    }
}
