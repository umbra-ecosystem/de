//! Local persistence.
//!
//! Two SQLite databases with different guarantees:
//!
//! - **`state.db`** holds data that exists nowhere else (claims, ordering, notes, checklists,
//!   time, overlays, drafts, the audit log). Losing it loses work, so its migrations are
//!   append-only and it is the one to back up.
//! - **`cache.db`** mirrors remote systems (Jira, PRs, pipelines). It can be deleted at any
//!   time and is rebuilt by the next sync.

pub mod audit;
pub mod jira_cache;
pub mod links;
mod migrations;
pub mod notes;
pub mod overlays;
pub mod restore;
mod sql;
pub mod tickets;
pub mod time;

#[cfg(test)]
mod domain_tests;

use std::path::{Path, PathBuf};

use eyre::{Context, eyre};
use rusqlite::Connection;

use crate::utils::get_project_dirs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Local-only data that cannot be reproduced.
    State,
    /// Disposable mirror of remote data.
    Cache,
}

impl Kind {
    pub fn file_name(self) -> &'static str {
        match self {
            Kind::State => "state.db",
            Kind::Cache => "cache.db",
        }
    }
}

/// An open, migrated SQLite database.
pub struct Store {
    kind: Kind,
    conn: Connection,
}

impl Store {
    /// Opens (creating if needed) the database of `kind` in the OS data directory.
    pub fn open_default(kind: Kind) -> eyre::Result<Self> {
        let dir = get_project_dirs()?.data_dir().to_path_buf();
        Self::open_in(&dir, kind)
    }

    /// Opens (creating if needed) the database of `kind` inside `dir`.
    pub fn open_in(dir: &Path, kind: Kind) -> eyre::Result<Self> {
        std::fs::create_dir_all(dir)
            .wrap_err_with(|| format!("Failed to create data directory {}", dir.display()))?;

        let path = dir.join(kind.file_name());
        let conn = Connection::open(&path)
            .wrap_err_with(|| format!("Failed to open database {}", path.display()))?;

        Self::init(kind, conn).wrap_err_with(|| format!("Failed to initialise {}", path.display()))
    }

    /// An in-memory database, for tests.
    pub fn open_in_memory(kind: Kind) -> eyre::Result<Self> {
        Self::init(kind, Connection::open_in_memory()?)
    }

    fn init(kind: Kind, mut conn: Connection) -> eyre::Result<Self> {
        // WAL lets the menubar app and the CLI read while the other writes.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        migrations::for_kind(kind)
            .to_latest(&mut conn)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to run database migrations")?;

        Ok(Self { kind, conn })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// The schema version, i.e. how many migrations have been applied.
    pub fn schema_version(&self) -> eyre::Result<u32> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    /// Deletes the cache database (and its WAL files) so the next sync rebuilds it. Never touches `state.db`.
    pub fn reset_cache(dir: &Path) -> eyre::Result<PathBuf> {
        let path = dir.join(Kind::Cache.file_name());
        for suffix in ["", "-wal", "-shm"] {
            let file = PathBuf::from(format!("{}{suffix}", path.display()));
            if file.exists() {
                std::fs::remove_file(&file)
                    .wrap_err_with(|| format!("Failed to remove {}", file.display()))?;
            }
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_valid() {
        migrations::for_kind(Kind::State).validate().unwrap();
        migrations::for_kind(Kind::Cache).validate().unwrap();
    }

    #[test]
    fn in_memory_store_is_migrated() {
        for kind in [Kind::State, Kind::Cache] {
            let store = Store::open_in_memory(kind).unwrap();
            assert!(store.schema_version().unwrap() >= 1);
        }
    }

    #[test]
    fn file_store_uses_wal_and_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in(dir.path(), Kind::State).unwrap();

        let mode: String = store
            .conn()
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        let fk: i64 = store
            .conn()
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();

        assert_eq!(mode, "wal");
        assert_eq!(fk, 1);
        assert!(dir.path().join("state.db").exists());
    }

    #[test]
    fn reopening_keeps_data_and_does_not_remigrate() {
        let dir = tempfile::tempdir().unwrap();

        let first = Store::open_in(dir.path(), Kind::State).unwrap();
        first
            .conn()
            .execute(
                "INSERT INTO app_meta (key, value) VALUES ('probe', '1')",
                [],
            )
            .unwrap();
        let version = first.schema_version().unwrap();
        drop(first);

        let second = Store::open_in(dir.path(), Kind::State).unwrap();
        assert_eq!(second.schema_version().unwrap(), version);
        let value: String = second
            .conn()
            .query_row("SELECT value FROM app_meta WHERE key = 'probe'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(value, "1");
    }

    #[test]
    fn state_and_cache_are_separate_files() {
        let dir = tempfile::tempdir().unwrap();
        let _state = Store::open_in(dir.path(), Kind::State).unwrap();
        let _cache = Store::open_in(dir.path(), Kind::Cache).unwrap();

        assert!(dir.path().join("state.db").exists());
        assert!(dir.path().join("cache.db").exists());
    }

    #[test]
    fn reset_cache_leaves_state_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let state = Store::open_in(dir.path(), Kind::State).unwrap();
        let cache = Store::open_in(dir.path(), Kind::Cache).unwrap();
        drop(cache);

        Store::reset_cache(dir.path()).unwrap();

        assert!(!dir.path().join("cache.db").exists());
        assert!(dir.path().join("state.db").exists());
        assert!(state.schema_version().unwrap() >= 1);
    }
}
