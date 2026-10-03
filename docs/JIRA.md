# Setting up Jira

`de` reads Jira through Atlassian's `acli`. It never stores credentials: `acli` owns the login. `de` only reads from Jira during a sync. Writes (comments, status changes) go through the confirmed write flow, never a sync.

## 1. Install and log in to `acli`

```sh
acli jira auth login      # OAuth in the browser, or an API token
acli jira auth status     # shows the site you are logged in to
```

Check that `de` sees it:

```sh
de providers check
```

The `jira (acli)` line should say `authenticated: yes` and `ready: ready`.

## 2. Configure (optional)

Nothing is required. With no `[jira]` section, `de` syncs with defaults: the site comes from `acli`, and the Review pool is every ticket in the review status updated in the last 30 days:

```
status = "In Review" AND updated >= -30d ORDER BY updated DESC
```

Add a `[jira]` section to narrow or change that. Edit `config.toml` in the `de` config directory (on macOS `~/Library/Application Support/Umbra.de/config.toml`):

```toml
[jira]
review_jql = "project = PROJ AND status = \"In Review\" AND assignee = currentUser()"
```

All keys are optional:

| Key | What it does | Default |
|---|---|---|
| `review_jql` | JQL for the pool of tickets in your Review column. This is what fills the app. | the query above, using your review status name |
| `account_id` | Your Jira account id. Comments that mention it are the "you were tagged" signal. | none (no mention signal) |
| `returned_jql` | JQL for tickets sent back to you. | `status = "<returned status>"` |
| `site` | Only overrides the host used for ticket links. Otherwise `de` asks `acli` (`auth status`) once per sync; if that fails it falls back to the API host, which is wrong for some sites — then set this. | from `acli` |
| `[jira.statuses]` | Your workflow's status names, below. | see below |

Quote status names inside JQL (`\"In Review\"` in a double-quoted TOML string, or a single-quoted TOML string with double quotes inside).

### Status names

You can edit all of these in the app under **Settings → Jira mapping**: click a field and type. Edited fields are marked "edited" and nothing is written until you press **Save** (or Enter). **Discard** puts the fields back to what is saved, and **Reset to defaults** clears every setting after asking. An empty field uses the default (shown greyed out). Lists (done and signed-off statuses) are comma-separated. Status names apply to the ticket list straight away; the review status, review query and account id are used from the next sync.

Or edit the file by hand. Map your workflow to the names `de` uses. Matching is case-insensitive. Anything not listed still shows, with Jira's own wording.

```toml
[jira.statuses]
review = "In Review"
alpha_testing = "Alpha Testing"
uat = "UAT"
returned = "Returned"
done = ["Done", "Closed"]
signed_off = ["UAT Passed"]   # optional: statuses that mean UAT was signed off
```

If you leave a key out, the default is `In Review`, `Alpha Testing`, `UAT`, `Returned`, `Done`. `signed_off` defaults to `done`.

### Finding your account id

```sh
acli jira workitem search --jql "assignee = currentUser()" --limit 1 --fields key,assignee --json
```

Copy `fields.assignee.accountId` from a ticket assigned to you.

## 3. Sync

```sh
de sync --force      # from the terminal
```

In the app, use **Sync now** (the app also syncs once when it opens). The status bar and the **Last sync** panel report, per source, something like `20 tickets, 4 comments`, or why it failed:

| Message | Meaning | Fix |
|---|---|---|
| `not logged in` | `acli` session expired | `acli jira auth login` |
| `not installed` | `acli` not found | install it and put it on `PATH` |
| `offline, cache kept` | network down | nothing; cached tickets stay visible |
| the pool is too big or too small | default query does not match your board | set `review_jql` |

A failed sync never deletes cached data. Tickets are removed from the cache only after a sync that completed fully shows they left the pool.

## Ticket detail and rich text

A sync reads each ticket once with `acli jira workitem view KEY --fields '*all'`. That one call gives the comments and the detail: description, type, reporter, created and updated times, labels, components, fix versions, sprint, epic, estimate, attachments, linked issues and subtasks. The description and comments keep their structure (headings, paragraphs and line breaks, bullet and numbered lists, checklists, code blocks, quotes and mentions). A field Jira has no value for is left out of the ticket page.

Only tickets that changed are read again. Search cannot return a ticket's update time, but it can filter on it, so a sync asks Jira once per hundred cached tickets which of them were `updated` since they were cached (`key in (...) AND updated >= "-25m"`, a relative time so the Jira site's timezone does not matter) and reads in full only those, plus tickets never read before. The sync report says how many changed. A ticket cached more than a week ago is read in full anyway, and `de sync --full` reads every ticket. If Jira cannot answer the "what changed" question, the tickets are read in full, so the cache may be slower to refresh but never stale.

The first sync, or one after a long time away, reads every ticket (about two seconds each). Comments cached by an earlier version show as plain lines until their ticket next changes or a `--full` sync.

## Automatic sync

While the app is open it syncs by itself every 10 minutes. Change that under **Settings → Sync** (Off, 5m, 10m, 30m, 1h) or in `config.toml`:

```toml
[sync]
interval_minutes = 10   # 0 turns it off
```

An automatic sync never overlaps a running one, is quiet (no toasts unless it starts failing, and then only once) and only appears in the audit log as `sync.auto` when it found changes or failed. After failures it waits longer (double each time, up to an hour) and goes back to normal after a success. A manual sync restarts the timer. The "Sync now" suggestion on Home appears when the last sync is two intervals old (at least six minutes).

## Sync logs

Every sync writes a raw log of what it asked Jira for and what came back: the `acli` commands, their exit codes and timings, and the output as `acli` printed it. One file per run, named by its start time in UTC (`sync-20261001-094012.log`; the app lists runs in your local time).

A run belongs to the workspace it was made in: its file goes into that workspace's own folder, `<data dir>/workspaces/<workspace>/logs` (on macOS `~/Library/Application Support/Umbra.de/workspaces/shop/logs`), and only that workspace lists it. A sync started with no workspace open is written to `<data dir>/logs`, which the app never lists.

- In the app, open **System → Sync logs** to pick a run and read it. `de sync` prints the path of its log when it finishes.
- The newest 20 runs of a workspace are kept; older ones are deleted when a new run starts. Change that in `config.toml`:

```toml
[logs]
keep = 50
```

- The files contain ticket titles and comments as Jira returned them (each output is cut at 256 KB). Review a log before sharing it.

## Diagnosing

- `de providers probe` prints the raw `acli` output if tickets look wrong. It can contain ticket titles, so review it before sharing.
- `cargo test -p de-gui -- --ignored live_` runs a read-only sync of your real Jira into throwaway databases and prints counts only.

## Not covered yet

GitHub (pull requests, Actions runs) is not wired. Until it is, the app's PR, review and shipping screens are simulated.
