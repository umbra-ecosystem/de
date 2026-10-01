# Workspace lifecycle

How a workspace is chosen, opened and closed in the app, and what the person sees while that happens. This is the
design the widgets are built to first (with a simulated store); the engine is wired to it afterwards.

## Principles

- **One workspace is open at a time.** Opening another closes the current one first. Several open at once may come
  later; nothing here should make that impossible, but nothing is built for it.
- **A workspace's data is its own.** Tickets, repos, review answers, repo locks and the audit log belong to the open
  workspace. Nothing of one shows in another, and the window keeps nothing of the old one (tabs, drafts, filters).
- **Nothing of a workspace without a workspace.** With none open, the screens that show a workspace's data cannot be
  reached. What does not need one stays: the settings, the sync logs and the list of workspaces.
- **Opening and closing are real work, so they are shown and not interrupted.** Starting services and reading git
  state takes seconds and can fail. The person sees each step, and the window cannot be used (or the step list
  closed) while it runs.
- **Failure is information, not a dead end.** A step that fails does not stop the others from running. Afterwards the
  person chooses: go back, try again, or continue anyway.

## The states

```
no workspace open ──select──▶ opening ──all steps ok──▶ open
        ▲                        │ a step failed
        │                        ▼
        │                 needs a decision ── continue anyway ──▶ open
        │                        │ back / retry
        │                        ▼
        └──────────────────── closing ◀──close / select another── open
```

`opening` and `closing` are the two sequences below. `needs a decision` is a sequence that has finished with at least
one failed step. The window is one of two things at any moment: the **landing page** (no workspace open) or the
**dock** (a workspace open). A sequence is a modal over whichever is showing.

## The landing page

The page before the dock, shown while no workspace is open. Centred, bare, no navigation, tabs or panels.

1. **A friendly introduction.** A welcome, one or two sentences of what the app is for and what happens next
   ("Pick a workspace and I'll start its services and check its repos, then show you what needs you."). Warm, not
   instructional.
2. **The saved workspaces, most recently used first, one line each.** The name, then its projects on the same line
   (`api-client · web · worker · docs`, cut with an ellipsis when long), then when it was last used and a dot when it
   is running. The folder is not shown: a workspace can span several.
3. **At most five rows.** More than five shows a **Show all** button that opens a modal with a search field and the
   whole list. Fewer than six shows no button.
4. **How to make one.** `de init`, one line, quiet.
5. **The tools**, one quiet line each (Jira, GitHub): a dot, the name, and when one is not ready what to run.

Choosing a workspace (a row, or a row in the modal) starts the **opening sequence**.

## The title bar menu

With a workspace open, the title bar shows its name. Clicking it opens the same list in a menu (search, the open
workspace with a check, the others by recency, `de init`, **Close workspace**). Choosing another workspace starts a
**switch**: the closing sequence for the open one, then the opening sequence for the new one, in one modal.

## The opening sequence

Run in the order the projects need to start (dependencies first, as `de start` does), one step at a time.

| # | Step | What it does | Result shown |
|---|------|--------------|--------------|
| 1 | Read the workspace | Read its configuration, check that every project folder exists. | `4 projects` |
| 2 | Git status, per repo | The current branch, ahead and behind, and the number of uncommitted changes, from git (no network). Captured as the workspace's state. | `develop, clean` / `web: develop, 2 uncommitted` |
| 3 | Start services, per project with a compose file | `docker compose up -d`, in dependency order. Projects without a compose file have no step. | `3 services` |
| 4 | Load the workspace | Open its own data (tickets, history, audit log) and read what is cached. | `12 tickets` |

Step 2 for every repo runs before any docker step, so the state captured is the state before the app touched
anything. A repo's compose step runs after its git status.

**Failures.** A failed step shows why and what to do, in words ("Docker is not running. Start Docker Desktop, then
retry."), and the sequence goes on with the rest. Steps that cannot make sense without the failed one (the services of
a project whose folder is missing) are skipped, with the reason.

**When it finishes.**

- All steps fine: a short beat showing "Ready", then the modal closes by itself and the dock appears with the
  workspace loaded (its Home, its tickets, its repos in the status bar).
- At least one step failed: the modal stays, with the outcome and three choices. **Back** abandons opening (what
  was started keeps running; opening again is safe, because starting is idempotent). **Retry** runs the sequence
  again from the top. **Continue anyway** opens the workspace as it is, with the failures noted on its Workspace page.

## The closing sequence

The opposite, in the reverse order (`de stop` order).

| # | Step | What it does |
|---|------|--------------|
| 1 | Check nothing is in progress | No ticket may be active in the workspace (it holds the repos) and no repo may be busy. If one is, the sequence stops here: the other steps are skipped, and the message says what to do ("PROJ-142 is active. Park it first."). |
| 2 | Stop services, per project with a compose file | `docker compose down`, in reverse dependency order. |
| 3 | Save the workspace | Record when it was last used; the window forgets its tabs, drafts and filters. |

Finished without failures: the modal closes by itself and the landing page appears. Blocked or failed: the modal
stays, with **Back** (the workspace stays open) and, for a failed service stop, **Close anyway**.

## The modal

- **While running**, it cannot be closed or dismissed: no close button, Escape does nothing, a click outside does
  nothing, and the keyboard shortcuts of the window are off. A line says to wait. There is no cancel, so a half-run
  sequence cannot be left half-run by accident.
- **Steps** are one line each: a mark (waiting, running with a spinner, done, warning, failed, skipped), the label, and
  on the right the result once there is one. The phase titles (`Closing shop`, `Opening hbt`) separate the two halves of
  a switch. A count (`4 of 9`) shows progress.
- **Finished with failures**, the footer holds the choices above and nothing else.

## What is real and what is simulated

The widgets run the whole flow over a simulated store (steps take a believable time; a switch can make Docker fail).
The engine behind it, when wired:

- Step 1 and the project order: `Workspace::load` and `ordered_projects(Order::Startup | Order::Shutdown)`.
- Step 2: `git2` reads (`GitRepo`), never the CLI, never the network.
- Step 3 and the services of closing: the `docker compose` calls of `Project::compose`, as `spin_up_workspace` and
  `spin_down_workspace` do today, one project at a time, each result reported to the sequence.
- Step 4: the workspace's own `state.db` and `cache.db` (one pair per workspace), opened at this point.
- The last-used time of a workspace is stored with it, and orders the lists.

Waiting on a process must never block the window: each step runs off the UI thread and reports back.
