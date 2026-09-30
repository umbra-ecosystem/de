//! Schema migrations, one ordered list per database.
//!
//! Append only: never edit a migration once it has run on a real database. Anything new
//! (PRs, pipelines, overlays, ...) is added as a new entry at the end of the list.
//!
//! The CHECK lists below mirror the text forms of the enums in `crate::domain`; a test in
//! `store::tests` fails if they drift apart.

use std::sync::LazyLock;

use rusqlite_migration::{M, Migrations};

use super::Kind;

const APP_META: &str = "CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;";

/// Tickets and local tracking: everything here is irreplaceable.
const STATE_TICKETS: &str = "
CREATE TABLE tickets (
    key           TEXT PRIMARY KEY NOT NULL,
    status        TEXT NOT NULL
                  CHECK (status IN ('claimed','reviewing','active','parked','integrated','done')),
    manual_order  INTEGER NOT NULL,
    kind_override TEXT CHECK (kind_override IS NULL OR kind_override IN ('normal','hotfix')),
    claimed_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
) STRICT;

-- At most one Active ticket, whatever the application code does.
CREATE UNIQUE INDEX tickets_single_active ON tickets (status) WHERE status = 'active';
CREATE INDEX tickets_by_status_order ON tickets (status, manual_order);

CREATE TABLE ticket_repos (
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    repo       TEXT NOT NULL,
    branch     TEXT,
    origin     TEXT NOT NULL CHECK (origin IN ('auto','manual','excluded')),
    PRIMARY KEY (ticket_key, repo)
) STRICT;

CREATE TABLE checklist_items (
    id         INTEGER PRIMARY KEY,
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    text       TEXT NOT NULL,
    done       INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
    UNIQUE (ticket_key, position)
) STRICT;

CREATE TABLE notes (
    id         INTEGER PRIMARY KEY,
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    body       TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX notes_by_ticket ON notes (ticket_key, created_at, id);

CREATE TABLE time_entries (
    id         INTEGER PRIMARY KEY,
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    ended_at   INTEGER CHECK (ended_at IS NULL OR ended_at >= started_at)
) STRICT;
CREATE INDEX time_entries_by_ticket ON time_entries (ticket_key);
-- At most one running timer overall.
CREATE UNIQUE INDEX time_entries_single_open ON time_entries ((1)) WHERE ended_at IS NULL;

-- No foreign key to tickets: the log must outlive the rows it describes.
CREATE TABLE audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         INTEGER NOT NULL,
    action     TEXT NOT NULL,
    ticket_key TEXT,
    repo       TEXT,
    details    TEXT NOT NULL CHECK (json_valid(details)),
    outcome    TEXT NOT NULL CHECK (outcome IN ('success','failure','skipped'))
) STRICT;
CREATE INDEX audit_log_by_ticket ON audit_log (ticket_key, id);

CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
";

/// Mirror of Jira tickets; disposable.
const CACHE_JIRA: &str = "
CREATE TABLE jira_tickets (
    key         TEXT PRIMARY KEY NOT NULL,
    title       TEXT NOT NULL,
    jira_status TEXT NOT NULL,
    priority    TEXT,
    assignee    TEXT,
    url         TEXT,
    raw_json    TEXT NOT NULL,
    fetched_at  INTEGER NOT NULL
) STRICT;
";

/// What activating a ticket changed, so deactivating (or recovering from a crash) can put
/// everything back using nothing but this database.
const STATE_ACTIVATION: &str = "
-- One row per repo an activation switched: where it was, so it can be restored.
CREATE TABLE activation_repos (
    ticket_key      TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    repo            TEXT NOT NULL,
    -- Order the repos were switched in; restoring goes the other way round.
    position        INTEGER NOT NULL,
    repo_dir        TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('ticket','baseline')),
    branch          TEXT NOT NULL,
    previous_branch TEXT,
    previous_commit TEXT,
    -- The stash made for a dirty tree: label and commit together, never an index.
    stash_label     TEXT,
    stash_commit    TEXT,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (ticket_key, repo),
    CHECK ((stash_label IS NULL) = (stash_commit IS NULL))
) STRICT;

-- The exact bytes of the consumer's composer files before the test overlay touched them.
CREATE TABLE overlay_backups (
    ticket_key     TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    repo           TEXT NOT NULL,
    repo_dir       TEXT NOT NULL,
    packages       TEXT NOT NULL CHECK (json_valid(packages)),
    composer_json  BLOB NOT NULL,
    -- NULL: composer.lock did not exist before the overlay.
    composer_lock  BLOB,
    -- What the overlay left behind, for verification.
    json_after     BLOB,
    lock_after     BLOB,
    -- Set once the files are back, so a retry after a failed `composer install` does not
    -- overwrite edits made in between.
    files_restored INTEGER NOT NULL DEFAULT 0 CHECK (files_restored IN (0, 1)),
    created_at     INTEGER NOT NULL,
    PRIMARY KEY (ticket_key, repo)
) STRICT;
-- A checkout carries at most one overlay at a time, whichever ticket applied it.
CREATE UNIQUE INDEX overlay_backups_one_per_dir ON overlay_backups (repo_dir);
";

/// The `uat` merge commits the app pushed, so pipelines can be found by commit. Written by
/// the integration flow (M5); read by sync to know which commits to look up. Irreplaceable:
/// the pushed merge commit is not recoverable from any remote system by ticket.
const STATE_UAT_MERGES: &str = "
CREATE TABLE uat_merges (
    id          INTEGER PRIMARY KEY,
    ticket_key  TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    -- The workspace project name, as in ticket_repos.repo (not the hosting path).
    repo        TEXT NOT NULL,
    branch      TEXT NOT NULL,
    commit_sha  TEXT NOT NULL,
    recorded_at INTEGER NOT NULL,
    UNIQUE (ticket_key, repo, commit_sha)
) STRICT;
CREATE INDEX uat_merges_by_repo ON uat_merges (repo, id);
";

/// Drafts of external writes composed locally (deploy comments, transitions). Irreplaceable
/// while unposted; a Posted row is what stops the same comment being posted twice.
const STATE_DRAFTS: &str = "
CREATE TABLE drafts (
    id         INTEGER PRIMARY KEY,
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('deploy_comment','transition')),
    body       TEXT NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('draft','posted','discarded')),
    created_at INTEGER NOT NULL,
    posted_at  INTEGER,
    remote_id  TEXT,
    CHECK ((status = 'posted') = (posted_at IS NOT NULL))
) STRICT;
CREATE INDEX drafts_by_ticket ON drafts (ticket_key, id);
";

/// What the integration flow knew when it recorded a `uat_merges` row.
const STATE_UAT_MERGE_DETAILS: &str = "
CREATE TABLE uat_merge_details (
    merge_id      INTEGER PRIMARY KEY REFERENCES uat_merges (id) ON DELETE CASCADE,
    -- 'merge': pushed by the app. 'already_in_uat': the ticket was already contained in uat,
    -- `commit_sha` is the uat tip that contains it and nothing was pushed.
    kind          TEXT NOT NULL CHECK (kind IN ('merge','already_in_uat')),
    ticket_branch TEXT NOT NULL,
    ticket_tip    TEXT NOT NULL,
    uat_before    TEXT NOT NULL
) STRICT;
";

/// The next-action engine's local memory: what the author said about suggestions, and the
/// marker of when a ticket was last reviewed.
const STATE_NEXT_ACTIONS: &str = "
-- Append-only history of responses; the latest row per suggestion id is the current one.
-- `facts_hash` identifies the facts the suggestion had when it was answered, so a
-- dismissal stops applying once they change materially. `suggestion_id` is derived from
-- ticket, rule and subject (see `next::Suggestion::make_id`), never from a rowid.
CREATE TABLE suggestion_responses (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    suggestion_id TEXT NOT NULL,
    ticket_key    TEXT,
    rule          TEXT NOT NULL,
    response      TEXT NOT NULL CHECK (response IN ('dismissed','snoozed','done')),
    reason        TEXT,
    snooze_until  INTEGER,
    facts_hash    TEXT NOT NULL,
    at            INTEGER NOT NULL,
    CHECK ((response = 'snoozed') = (snooze_until IS NOT NULL))
) STRICT;
CREATE INDEX suggestion_responses_by_id ON suggestion_responses (suggestion_id, id);

-- When a ticket's review was finished, and the branch tips it covered (JSON object of
-- project name to commit). Absent: not reviewed (or the review was restarted).
CREATE TABLE ticket_reviews (
    ticket_key  TEXT PRIMARY KEY NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    reviewed_at INTEGER NOT NULL,
    heads       TEXT NOT NULL DEFAULT '{}'
) STRICT;
";

/// Mirror of Bitbucket PRs and pipelines and of Jira comments, plus per-source sync
/// bookkeeping; all disposable.
///
/// CHECK lists mirror `providers::PrState` (a test in `store::tests` keeps them in sync).
const CACHE_PROVIDERS: &str = "
CREATE TABLE prs (
    repo               TEXT NOT NULL,
    id                 INTEGER NOT NULL,
    title              TEXT NOT NULL,
    state              TEXT NOT NULL CHECK (state IN ('open','merged','declined','superseded')),
    source_branch      TEXT NOT NULL,
    destination_branch TEXT NOT NULL,
    author             TEXT NOT NULL,
    url                TEXT NOT NULL,
    updated_at         INTEGER NOT NULL,
    fetched_at         INTEGER NOT NULL,
    PRIMARY KEY (repo, id)
) STRICT;
CREATE INDEX prs_by_state ON prs (repo, state);

CREATE TABLE pr_reviewers (
    repo              TEXT NOT NULL,
    pr_id             INTEGER NOT NULL,
    account           TEXT NOT NULL,
    approved          INTEGER NOT NULL CHECK (approved IN (0, 1)),
    changes_requested INTEGER NOT NULL CHECK (changes_requested IN (0, 1)),
    PRIMARY KEY (repo, pr_id, account),
    FOREIGN KEY (repo, pr_id) REFERENCES prs (repo, id) ON DELETE CASCADE
) STRICT;

CREATE TABLE pr_comments (
    repo        TEXT NOT NULL,
    pr_id       INTEGER NOT NULL,
    id          INTEGER NOT NULL,
    author      TEXT NOT NULL,
    body        TEXT NOT NULL,
    inline_path TEXT,
    inline_line INTEGER,
    inline_side TEXT CHECK (inline_side IS NULL OR inline_side IN ('old','new')),
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (repo, pr_id, id),
    FOREIGN KEY (repo, pr_id) REFERENCES prs (repo, id) ON DELETE CASCADE
) STRICT;

CREATE TABLE pipeline_runs (
    repo         TEXT NOT NULL,
    id           TEXT NOT NULL,
    number       INTEGER,
    state        TEXT NOT NULL,
    branch       TEXT NOT NULL,
    commit_sha   TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    completed_at INTEGER,
    url          TEXT NOT NULL,
    fetched_at   INTEGER NOT NULL,
    PRIMARY KEY (repo, id)
) STRICT;
CREATE INDEX pipeline_runs_by_commit ON pipeline_runs (repo, commit_sha);
CREATE INDEX pipeline_runs_by_branch ON pipeline_runs (repo, branch, created_at);

CREATE TABLE pipeline_steps (
    repo                   TEXT NOT NULL,
    run_id                 TEXT NOT NULL,
    position               INTEGER NOT NULL,
    name                   TEXT NOT NULL,
    state                  TEXT NOT NULL,
    deployment_environment TEXT,
    PRIMARY KEY (repo, run_id, position),
    FOREIGN KEY (repo, run_id) REFERENCES pipeline_runs (repo, id) ON DELETE CASCADE
) STRICT;

CREATE TABLE jira_comments (
    ticket_key         TEXT NOT NULL,
    id                 TEXT NOT NULL,
    author_account_id  TEXT NOT NULL,
    author_name        TEXT NOT NULL,
    body_text          TEXT NOT NULL,
    created_at         INTEGER NOT NULL,
    fetched_at         INTEGER NOT NULL,
    PRIMARY KEY (ticket_key, id)
) STRICT;

CREATE TABLE jira_comment_mentions (
    ticket_key TEXT NOT NULL,
    comment_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    PRIMARY KEY (ticket_key, comment_id, account_id),
    FOREIGN KEY (ticket_key, comment_id) REFERENCES jira_comments (ticket_key, id) ON DELETE CASCADE
) STRICT;
CREATE INDEX jira_comment_mentions_by_account ON jira_comment_mentions (account_id);

-- One row per sync source (`jira`, `bitbucket:workspace/slug`).
CREATE TABLE sync_state (
    source          TEXT PRIMARY KEY NOT NULL,
    last_ok_at      INTEGER,
    last_attempt_at INTEGER NOT NULL,
    last_error      TEXT
) STRICT;
";

static STATE: LazyLock<Migrations<'static>> = LazyLock::new(|| {
    Migrations::new(vec![
        M::up(APP_META),
        M::up(STATE_TICKETS),
        M::up(STATE_ACTIVATION),
        M::up(STATE_UAT_MERGES),
        M::up(STATE_DRAFTS),
        M::up(STATE_UAT_MERGE_DETAILS),
        M::up(STATE_NEXT_ACTIONS),
    ])
});

static CACHE: LazyLock<Migrations<'static>> = LazyLock::new(|| {
    Migrations::new(vec![
        M::up(APP_META),
        M::up(CACHE_JIRA),
        M::up(CACHE_PROVIDERS),
    ])
});

pub(super) fn for_kind(kind: Kind) -> &'static Migrations<'static> {
    match kind {
        Kind::State => &STATE,
        Kind::Cache => &CACHE,
    }
}
