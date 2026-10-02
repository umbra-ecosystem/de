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
pub mod drafts;
pub mod jira_cache;
pub mod jira_comments;
pub mod jira_details;
pub mod links;
mod migrations;
pub mod notes;
pub mod overlays;
pub mod pipelines;
pub mod prs;
pub mod restore;
pub mod reviews;
mod sql;
pub mod suggestion_responses;
pub mod sync_state;
pub mod tickets;
pub mod time;
pub mod uat_details;
pub mod uat_merges;

#[cfg(test)]
mod domain_tests;
#[cfg(test)]
mod next_tests;
#[cfg(test)]
mod provider_tests;

use std::path::{Path, PathBuf};

use eyre::{Context, eyre};
use rusqlite::Connection;

use crate::{config::Config, project::Project, types::Slug, utils::get_project_dirs};

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

/// The directory holding one workspace's own `state.db` and `cache.db`.
///
/// A workspace's tickets, notes, locks and audit log are its own; nothing of one is ever read
/// from another's files.
pub fn workspace_dir(workspace: &Slug) -> eyre::Result<PathBuf> {
    Ok(get_project_dirs()?
        .data_dir()
        .join("workspaces")
        .join(workspace.as_str()))
}

/// The workspace this process acts on: the one the current folder belongs to, else the one
/// selected in `config.toml`, else none (the shared data directory still serves it).
fn scoped_workspace_name() -> eyre::Result<Option<Slug>> {
    if let Some(project) = Project::current()? {
        return Ok(Some(project.manifest().project().workspace.clone()));
    }
    Ok(Config::load()?.get_active_workspace().cloned())
}

/// An open, migrated SQLite database.
pub struct Store {
    kind: Kind,
    conn: Connection,
}

impl Store {
    /// Opens (creating if needed) the database of `kind` for `workspace`'s own data.
    ///
    /// The first time a workspace opens, the shared databases from before workspaces were
    /// isolated are moved into it (see [`adopt_shared_databases`]), so nothing is lost.
    pub fn open_for_workspace(workspace: &Slug, kind: Kind) -> eyre::Result<Self> {
        let dirs = get_project_dirs()?;
        let data = dirs.data_dir();
        let active = Config::load()?.get_active_workspace().cloned();
        Self::open_for_workspace_in(data, workspace, active.as_ref(), kind)
    }

    /// [`Store::open_for_workspace`] with the paths handed in, so the adoption rules are
    /// testable without touching the real data directory.
    fn open_for_workspace_in(
        data_root: &Path,
        workspace: &Slug,
        active: Option<&Slug>,
        kind: Kind,
    ) -> eyre::Result<Self> {
        let target = data_root.join("workspaces").join(workspace.as_str());
        adopt_shared_databases(data_root, &target, workspace, active)?;
        Self::open_in(&target, kind)
    }

    /// Opens (creating if needed) the database of `kind` for the workspace this invocation
    /// works in (see [`scoped_workspace_name`]), or the shared data directory when there is
    /// none: a person with no workspace yet keeps working exactly as before.
    pub fn open_scoped(kind: Kind) -> eyre::Result<Self> {
        match scoped_workspace_name()? {
            Some(workspace) => Self::open_for_workspace(&workspace, kind),
            None => Self::open_in(get_project_dirs()?.data_dir(), kind),
        }
    }

    /// `workspace`'s databases when there is one, else [`Store::open_scoped`]. This is what a
    /// command with a `-w` flag hands its store opens to.
    pub fn open_scoped_for(workspace: Option<&Slug>, kind: Kind) -> eyre::Result<Self> {
        match workspace {
            Some(workspace) => Self::open_for_workspace(workspace, kind),
            None => Self::open_scoped(kind),
        }
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

/// Move the shared databases (one pair in the data directory, from before workspaces each had
/// their own) into the workspace that is opening, exactly once.
///
/// Rules, so no history is ever lost or handed to the wrong workspace:
///
/// - Only when the target has no `state.db` yet, and the shared one exists.
/// - Only for the workspace `[active] workspace` points at; when nothing is selected, the
///   first workspace to open adopts it. Any other workspace gets its own fresh files and the
///   shared pair waits where it is.
/// - A file that already exists at the target is never overwritten.
fn adopt_shared_databases(
    data_root: &Path,
    target: &Path,
    workspace: &Slug,
    active: Option<&Slug>,
) -> eyre::Result<()> {
    if target.join(Kind::State.file_name()).exists() {
        return Ok(());
    }
    if !data_root.join(Kind::State.file_name()).exists() {
        return Ok(());
    }
    if let Some(active) = active
        && active != workspace
    {
        return Ok(());
    }

    std::fs::create_dir_all(target)
        .wrap_err_with(|| format!("Failed to create data directory {}", target.display()))?;

    for kind in [Kind::State, Kind::Cache] {
        for suffix in ["", "-wal", "-shm"] {
            let name = format!("{}{suffix}", kind.file_name());
            let from = data_root.join(&name);
            let to = target.join(&name);
            if !from.exists() || to.exists() {
                continue;
            }
            std::fs::rename(&from, &to).wrap_err_with(|| {
                format!(
                    "The existing data could not be moved into the workspace '{}': {} could not be renamed",
                    workspace,
                    from.display()
                )
            })?;
        }
    }
    Ok(())
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

    #[test]
    fn the_drafts_and_merge_detail_migrations_upgrade_a_v4_database_keeping_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            migrations::for_kind(Kind::State)
                .to_version(&mut conn, 4)
                .unwrap();
            conn.execute(
                "INSERT INTO tickets (key, status, manual_order, claimed_at, updated_at)
                 VALUES ('PROJ-1', 'integrated', 0, 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO uat_merges (ticket_key, repo, branch, commit_sha, recorded_at)
                 VALUES ('PROJ-1', 'web', 'uat', 'abc', 5)",
                [],
            )
            .unwrap();
        }

        let store = Store::open_in(dir.path(), Kind::State).unwrap();
        assert!(store.schema_version().unwrap() >= 6);
        let key: crate::domain::TicketKey = "PROJ-1".parse().unwrap();

        // The old merge is intact and readable without details.
        let merges = uat_details::list_for_ticket(&store, &key).unwrap();
        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].merge.commit, "abc");
        assert!(merges[0].details.is_none());

        // The new tables work, and their CHECK lists accept exactly the enums' text forms.
        for kind in drafts::DraftKind::ALL {
            let d = drafts::create(&store, &key, *kind, "body", 1).unwrap();
            assert_eq!(d.kind, *kind);
        }
        for status in drafts::DraftStatus::ALL {
            store
                .conn()
                .execute(
                    "INSERT INTO drafts (ticket_key, kind, body, status, created_at, posted_at)
                     VALUES ('PROJ-1', 'transition', 'b', ?1, 1, CASE WHEN ?1 = 'posted' THEN 1 END)",
                    [status.as_str()],
                )
                .unwrap();
        }
        assert!(
            store
                .conn()
                .execute(
                    "INSERT INTO drafts (ticket_key, kind, body, status, created_at)
                     VALUES ('PROJ-1', 'bogus', 'b', 'draft', 1)",
                    [],
                )
                .is_err()
        );
        // Posted needs a time, a draft must not have one.
        assert!(
            store
                .conn()
                .execute(
                    "INSERT INTO drafts (ticket_key, kind, body, status, created_at)
                     VALUES ('PROJ-1', 'transition', 'b', 'posted', 1)",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn upgrading_from_every_earlier_version_keeps_the_data() {
        let latest = Store::open_in_memory(Kind::State)
            .unwrap()
            .schema_version()
            .unwrap() as usize;
        for from in 1..latest {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.db");
            {
                let mut conn = Connection::open(&path).unwrap();
                migrations::for_kind(Kind::State)
                    .to_version(&mut conn, from)
                    .unwrap();
                if from >= 2 {
                    conn.execute(
                        "INSERT INTO tickets (key, status, manual_order, claimed_at, updated_at)
                         VALUES ('PROJ-1', 'active', 0, 1, 1)",
                        [],
                    )
                    .unwrap();
                    conn.execute(
                        "INSERT INTO time_entries (ticket_key, started_at) VALUES ('PROJ-1', 1)",
                        [],
                    )
                    .unwrap();
                }
            }

            let store = Store::open_in(dir.path(), Kind::State).unwrap();
            assert_eq!(store.schema_version().unwrap() as usize, latest);
            if from >= 2 {
                let status: String = store
                    .conn()
                    .query_row("SELECT status FROM tickets WHERE key = 'PROJ-1'", [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                assert_eq!(status, "active");
            }
        }
    }

    fn slug(s: &str) -> Slug {
        s.parse().unwrap()
    }

    /// One row proving the data is this workspace's, not somebody else's.
    fn probe(store: &Store, value: &str) {
        store
            .conn()
            .execute(
                "INSERT INTO app_meta (key, value) VALUES ('probe', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [value],
            )
            .unwrap();
    }

    fn probe_of(store: &Store) -> Option<String> {
        store
            .conn()
            .query_row("SELECT value FROM app_meta WHERE key = 'probe'", [], |r| {
                r.get(0)
            })
            .ok()
    }

    #[test]
    fn each_workspace_keeps_its_own_databases() {
        let root = tempfile::tempdir().unwrap();
        let shop = Store::open_for_workspace_in(root.path(), &slug("shop"), None, Kind::State).unwrap();
        probe(&shop, "shop's data");
        drop(shop);
        let hbt = Store::open_for_workspace_in(root.path(), &slug("hbt"), None, Kind::State).unwrap();
        probe(&hbt, "hbt's data");

        assert_eq!(
            probe_of(
                &Store::open_for_workspace_in(root.path(), &slug("shop"), None, Kind::State)
                    .unwrap()
            )
            .as_deref(),
            Some("shop's data"),
            "each workspace reads back only its own row"
        );
        assert_eq!(
            probe_of(
                &Store::open_for_workspace_in(root.path(), &slug("hbt"), None, Kind::State)
                    .unwrap()
            )
            .as_deref(),
            Some("hbt's data")
        );
        assert!(root.path().join("workspaces").join("shop").join("state.db").exists());
        assert!(root.path().join("workspaces").join("hbt").join("state.db").exists());
    }

    #[test]
    fn the_first_workspace_to_open_takes_over_the_shared_databases() {
        let root = tempfile::tempdir().unwrap();
        // The shared pair, as it was before workspaces were isolated.
        {
            let shared = Store::open_in(root.path(), Kind::State).unwrap();
            probe(&shared, "years of history");
        }

        let opened =
            Store::open_for_workspace_in(root.path(), &slug("shop"), None, Kind::State).unwrap();
        assert_eq!(probe_of(&opened).as_deref(), Some("years of history"));
        assert!(
            !root.path().join("state.db").exists(),
            "the shared file was moved, not copied"
        );

        // Nothing is left for the next workspace: it starts with its own empty files.
        let other = Store::open_for_workspace_in(root.path(), &slug("hbt"), None, Kind::State).unwrap();
        assert_eq!(probe_of(&other), None);
    }

    #[test]
    fn the_shared_databases_wait_for_the_selected_workspace() {
        let root = tempfile::tempdir().unwrap();
        {
            let shared = Store::open_in(root.path(), Kind::State).unwrap();
            probe(&shared, "history");
        }
        let active = slug("shop");

        // Another workspace opening first must not swallow them.
        let wrong =
            Store::open_for_workspace_in(root.path(), &slug("hbt"), Some(&active), Kind::State)
                .unwrap();
        assert_eq!(probe_of(&wrong), None);
        assert!(root.path().join("state.db").exists(), "still there");

        // The selected workspace gets them.
        let right =
            Store::open_for_workspace_in(root.path(), &slug("shop"), Some(&active), Kind::State)
                .unwrap();
        assert_eq!(probe_of(&right).as_deref(), Some("history"));
        assert!(!root.path().join("state.db").exists());
    }

    #[test]
    fn opening_a_workspace_again_never_re_adopts_anything() {
        let root = tempfile::tempdir().unwrap();
        {
            let shared = Store::open_in(root.path(), Kind::State).unwrap();
            probe(&shared, "old");
        }
        let first =
            Store::open_for_workspace_in(root.path(), &slug("shop"), None, Kind::State).unwrap();
        probe(&first, "new");
        drop(first);

        // A shared database appears again (say, an old backup is put back); the workspace's
        // own data wins and is never overwritten.
        {
            let shared = Store::open_in(root.path(), Kind::State).unwrap();
            probe(&shared, "something else");
        }
        let again =
            Store::open_for_workspace_in(root.path(), &slug("shop"), None, Kind::State).unwrap();
        assert_eq!(probe_of(&again).as_deref(), Some("new"));
    }

    #[test]
    fn the_wal_files_move_with_their_database() {
        let root = tempfile::tempdir().unwrap();
        {
            let shared = Store::open_in(root.path(), Kind::State).unwrap();
            probe(&shared, "history");
        }
        // A sidecar left behind, as a database that was not closed cleanly would leave one.
        std::fs::write(root.path().join("state.db-wal"), b"").unwrap();
        let _shop = Store::open_for_workspace_in(root.path(), &slug("shop"), None, Kind::State)
            .unwrap();
        assert!(!root.path().join("state.db-wal").exists(), "taken along");
        assert!(root
            .path()
            .join("workspaces")
            .join("shop")
            .join("state.db-wal")
            .exists());
    }
}
