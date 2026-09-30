# GOAL

`de` becomes a **personal, ticket-centred work desk that lives in the macOS menubar**. A Jira ticket is the unit of work. Everything that belongs to it (repos, branches, PRs, pipelines, review notes, test checklist, time) is visible in one place, and the app **suggests the next action** for every ticket, carrying it out in one confirmed step.

Audience: one developer (the author), 3–8 repos, macOS only. Not a team tool. Generality is a non-goal unless it is free.

## The real workflow

The author is the **reviewer, local tester of the main flow, and integrator**. Others author most tickets. Full alpha testing and UAT are done by others.

### Lifecycle of a ticket

```
Jira: In Review ──► (review + local test + integrate) ──► Alpha Testing ──► UAT ──► sign-off
                                                              │
                                                              └─► Returned ──► (back to Review)
```

1. **Appears.** The ticket is in the board's *Review* column. The pool is every ticket in that column; nothing in Jira says whose it is, so the author **claims** the ones they will handle. A ticket in *Returned* is only relevant if the author is **@mentioned in a Jira comment**, or when it comes back to *Review*.
2. **Review.** Read the code without checking anything out: diffs come from git objects, not the working tree, so **any number of tickets can be under review at once**. Comments go on the Bitbucket PR (inline). **Approval is not given here**: the PR can only be approved once testing, including UAT after alpha, is complete. PRs are never merged through Bitbucket in this flow.
3. **Local test.** Exactly **one ticket is active** at a time (each repo can only be on one branch and the stack runs one version). Activating a ticket:
   - switches every repo the ticket touches to its branch, in place, stashing whatever was there. A ticket **touches only the repos that have a matching branch**; every **other repo goes to its base branch** (`develop` by default, configurable per repo) and is brought up to date, so the whole stack is the ticket's code on a known baseline;
   - applies the **test overlay** only where it is needed: when a repo that **provides** a package (for example the API client) has the ticket branch, each repo that **consumes** it gets a Composer **`path` repository (symlink) to the provider's local checkout**, with the package's version constraint set to `*`, then `composer update <package>`, and the configured rebuild steps run (UI build, etc.). Repos that don't consume such a package are never touched;
   - assumes the workspace (Compose) is already up;
   - gives the ticket a **test checklist and notes**.

   Tickets the author started testing and wants to resume are **parked**, with their notes and checklist. Deactivating restores the previous state and **reverts the overlay**.
4. **Integrate.** For each repo the ticket touches (repos without a ticket branch are skipped; there is nothing to merge): fetch, pull `uat`, merge the ticket branch, run a local check (manual, varies by repo), **push `uat`**. The push triggers a pipeline with a **deployment step** (alpha). `uat` is **long-lived and accumulates tickets**, so merges can conflict with other tickets' work. The app records the **`uat` merge commit it pushed** for each repo.
5. **Watch pipelines.** A repo's pipeline for the ticket is identified **by the merge commit** (the run for that commit, or any later run that contains it). "Deployed" means the deployment step succeeded.
6. **Announce.** When every repo's pipeline has deployed, the app **drafts a Jira comment**: each repo with its pipeline id and link (a ticket can span repos). The author confirms, it posts, then the ticket is transitioned **In Review → Alpha Testing** (also confirmed).
7. **Afterwards.** Track alpha/UAT sign-off. If the ticket branch gets **new commits not in `uat`** (fix after alpha), suggest re-merging, which produces a new pipeline and a new comment. When testing is fully complete, suggest **approving the PRs**. If the ticket is **Returned**, it is ignored unless the author is mentioned or it returns to Review.

Out of scope for the first versions: reverting a ticket out of `uat`, and following tickets to production/main.

## Decisions

| Topic | Decision |
|---|---|
| Unit of work | **Ticket** (Jira key). Workspace is the container of repos and services. |
| Role | Reviewer + local tester + integrator |
| Shape | **Always-on macOS menubar app**: background sync, notifications when new suggestions appear, window on demand. GUI is the primary interface. |
| Interfaces | GUI (GPUI via `gpui-kit`) first. The core is headless and testable. The CLI stays a thin surface (`start`, `stop`, `compose`, tasks, later `next`). |
| Ticket source | **Jira Cloud** |
| Ticket pool | Board *Review* column, claimed manually; *Returned* only when @mentioned |
| Ordering | Jira **priority** field plus **manual ordering** |
| Ticket ↔ repos | **Auto-discover by key** (local and remote branches, PRs) **plus manual add/exclude**, because duplicate and stale branches exist |
| Branch naming | Ticket key **anywhere in the name**, matched case-insensitively |
| Code host | **Bitbucket now** (Cloud vs Data Center to confirm), GitHub later including its pipelines |
| Data access | **Prefer existing CLIs, fall back to REST**, behind a full sync engine with a local cache. `de` never handles credentials: each CLI owns its login and secret storage. |
| Review | **Full in-app review**: diffs, inline comments, later approve |
| Working trees | **Switch in place, stash by default**; one active ticket plus a parked list |
| Repos without the ticket branch | **Fall back to the repo's base branch** (`develop` by default), fetched and fast-forwarded; skipped for review and integration |
| Composer overlay | **Configured per repo** (provider/consumer mapping); applied only when the providing repo has the ticket branch |
| Local time | Tracked **per ticket, locally** (no Jira worklog) |
| Automation | **Suggest everything, execute only on confirmation**; only reads and local-safe housekeeping run automatically |
| Order of work | **Solid foundation first**, then UI |

## Write policy

Local tracking state never leaves the app. The app may make these external writes, always as a deliberate action, never in the background:

- **Git:** push `uat` (this triggers deployment).
- **Jira:** deploy comment, In Review → Alpha Testing transition.
- **Bitbucket/GitHub:** PR review comments, approve / request changes (later), trigger or re-run pipelines.

Rules:

- All external writes go through **one gateway** that records them in a local **audit log**, including the facts that motivated them.
- The **deploy comment is a draft first**, listing each repo with its pipeline id/link; posting requires confirmation.
- **Pushing `uat` is the most consequential action** (it deploys). It requires: a clean integration state, the overlay reverted (see safety below), and an explicit confirm showing exactly what will be pushed per repo.
- Nothing posts automatically, including when pipelines finish. The app may notify that a draft is ready.
- Reads are free; a failed read must never block local work (offline is a supported state).

### Per-repo configuration

Repos differ, so behaviour is declared per project (in that project's `de.toml`, next to the existing `[project]` and `[tasks]`), not decided per ticket. Illustrative shape, to be settled in M1/M3:

```toml
[branches]
base = "develop"   # fallback when the ticket has no branch here; also the review baseline
uat  = "uat"       # integration branch

# Only in repos that CONSUME a package built from another workspace repo.
[overlay.composer]
packages = { "acme/api-client" = "api-client" }   # composer package -> providing workspace project
rebuild  = ["composer-update", "build-ui"]        # tasks run after the overlay is applied

# Only in repos that need a rebuild when *they* are on a ticket branch (no package involved).
[activate]
after = ["build-ui"]
```

How it resolves for an active ticket:

- **Provider has the ticket branch, consumer configured:** the consumer's `composer.json` gets a `repositories` entry `{ "type": "path", "url": "<provider checkout>", "options": { "symlink": true } }` and its `require` constraint for the package becomes `*`; then `composer update <package>` and the `rebuild` tasks run. The path is derived from the provider's registered project directory, so it needs no manual entry.
- **Provider on base (no ticket branch):** nothing is changed in the consumer, even though it is configured.
- **Repo with no `overlay` section:** never touched.
- Rebuild steps are ordinary `de` tasks, so they reuse the task proxy and can differ per repo.
- Defaults come from the existing workspace `default_branch` setting where present, then `develop`.

### Safety: the test overlay must never reach `uat`

The `composer.json` change for local testing (the path repository and the `*` constraint) is deliberately temporary and must **never be committed or pushed**. It also changes **`composer.lock`** and the consumer's `vendor/` (the package becomes a symlink), so all three are part of the overlay. So:

- The app records exactly what the overlay changed (`composer.json` and `composer.lock` contents before and after) and reverts them on deactivate, then re-runs `composer install` so `vendor/` no longer points at the symlink.
- Integrating (merging and pushing `uat`) must not happen from a tree carrying the overlay. Do it in a **temporary worktree** so it also does not disturb the active test checkout, or refuse until the overlay is reverted.
- Before any push, a guard verifies that nothing from the overlay is in the commits being pushed.

## Integrations and CLI coverage

Checked against published docs; nothing was runnable locally (`acli`, `bkt`, `gh` are not installed on this machine yet).

| System | Tool | Covers | Gaps / to verify |
|---|---|---|---|
| Jira | `acli jira workitem` (Atlassian's) | view, search, edit, assign, transition, comment create/list/update, link | JSON output and pagination unconfirmed. **Can it list comments with @mention data?** (needed for "Returned, tagged".) Board/column and priority queries unverified. |
| Bitbucket | `bkt` (community, [avivsinai/bitbucket-cli](https://github.com/avivsinai/bitbucket-cli), Go, MIT) | PR list/view/create/merge/approve/request-changes, comment resolve/reopen/delete, pipelines list/view/run/rerun/logs, `--json`/`--yaml`, OS-keychain credentials, Cloud and Data Center | **Inline comment creation and PR diff unverified.** Pipeline lookup by commit unverified. Community project. |
| GitHub (later) | `gh` | PRs, reviews, checks, runs, rerun; `gh api` for the rest | Line-level review comments need `gh api`. |

Consequences:

- Each CLI's login is done once by the user. `de` only checks that the CLI is installed, authenticated and at a tested version, and fails clearly otherwise. Parse **structured output only**, pin versions.
- **Diffs come from git, not the host.** The repos are local; the CLI is only for PR metadata, comments and pipelines.
- The one real risk is **posting inline review comments**. If `bkt` cannot, options are: contribute it upstream, use a raw-API passthrough if it has one, or make that single call over REST (reintroducing credentials for that path).
- Providers sit behind traits (`TicketProvider`, `CodeHost`, `Pipelines`) so an adapter can be a CLI wrapper or a REST client. Spawning a process per fetch is fine on demand but wrong for a sync hot path; Jira may end up on REST if `acli` is too slow or too lossy. That is an adapter swap.

## Next-action engine

Answers: *given everything known about my tickets, what should I do now?*

- A **pure function** `(store snapshot, now) -> Vec<Suggestion>`; no I/O, so every rule is unit-tested against fixture snapshots.
- A **Suggestion** has: ticket, a typed **Action**, a human-readable **reason**, **priority**, and whether it is safe to auto-run.
- **Actions are typed values** executed by the write gateway (external) or a local executor (git, Compose, tasks). The UI never builds raw commands.
- Recomputed after each sync or local change; only the user's responses (dismissed, snoozed, done, with an optional reason) are persisted.
- **Deterministic and explainable first.** An LLM layer (summarising, drafting text) is a possible later add-on.

| Level | Examples | Behaviour |
|---|---|---|
| Automatic | sync, `git fetch`, recompute, notify | Runs on its own; never changes a working tree or an external system |
| One-click, local | activate ticket, apply/revert overlay, rebuild, park | Suggested; runs after a click; safe by default (stash, no force) |
| Confirmed, external | push `uat`, deploy comment, transition, PR comments, trigger pipeline | Shows exactly what will be sent; confirming is the confirmation; audited |

**Rules that follow from the flow** (to be tuned on real data):

- *New ticket in Review, not claimed:* suggest claiming (ordered by priority, then manual order).
- *Claimed, not reviewed:* suggest starting the review (open diffs).
- *New commits since you reviewed:* suggest re-reviewing.
- *Reviewed, not yet tested:* suggest activating it for local test (if none active); if one is active, suggest finishing or parking it first.
- *Active ticket, API client changed:* suggest applying the composer overlay and rebuild steps.
- *Active ticket, checklist complete:* suggest integrating to `uat`.
- *Integrating, `uat` conflicts:* show which ticket's changes conflict, do not proceed.
- *Overlay still applied when integrating or deactivating:* block and suggest reverting.
- *Pushed, pipeline running or failed:* show status; on failure suggest viewing the step log or re-running.
- *All repos deployed:* suggest the deploy-comment draft, then the transition to Alpha Testing.
- *Ticket branch has commits not in `uat`:* suggest re-merging.
- *Returned and you were @mentioned:* surface the ticket and the comment.
- *Returned ticket back in Review:* treat as new.
- *Testing complete (UAT signed off):* suggest approving the PRs.
- *Stale active or parked ticket, or dirty tree on a ticket you are leaving:* suggest parking, stashing or committing.
- *Nothing actionable:* top of the priority-ordered queue.

Safety: suggestions never bypass the write policy; "do all" is limited to local, reversible actions; every executed suggestion is logged with the facts that triggered it.

## Architecture

Cargo workspace (already in place):

- `crates/core` (`de-core`): domain and orchestration, no UI. Grows to include:
  - **Domain model**: Ticket, Repo, Branch link, PR, Pipeline run, `uat` merge record, Overlay record, Checklist/Note, Time entry, Suggestion response.
  - **Store**: local SQLite (model, sync cache, audit log). Distinguishes **remote-mirrored data** (disposable) from **local-only data** (claims, ordering, notes, checklists, time, overlays, audit log, drafts; not reproducible, so migrations and backups matter).
  - **Git layer**: structured status, key matching across repos (local and remote, with duplicate/stale handling and manual override), safe in-place switch with stash, temporary worktrees for integration, reading diffs from git objects. Prefer `git2`/`gix` over parsing porcelain.
  - **Overlay engine**: apply, record, revert and verify test-only changes.
  - **Sync engine**: per-provider incremental sync; refresh on demand and on a timer; rate limiting; offline. Remote is the source of truth; local tracking is a separate overlay.
  - **Write gateway** and **next-action engine** as above.
  - **Environment**: existing Compose orchestration and task proxy (rebuild steps are just tasks).
- `crates/cli` (`de`): thin commands over core.
- `crates/gui` (`de-gui`): the menubar app over core. No business logic. Owns the always-on sync loop and notifications.

## Milestones

Foundation first. Each milestone is usable and tested headlessly before the next starts.

- **M0: Done.** Workspace split, command surface slimmed, task proxy (shell execution, exit codes propagate), clippy clean.
- **M1: Domain and store.** Ticket/repo/branch model, SQLite with migrations, local state (claims, ordering, notes, checklist, time), audit log. Import current workspace config.
- **M2: Git layer.** Structured status, key matching with manual override, safe in-place switch with stash/restore, diff-from-objects, temporary worktrees, restore the uncommitted/unpushed guard on `stop`.
- **M3: Active ticket and local test.** Activate/park/restore, overlay engine (composer pointing at the ticket branch, rebuild tasks) with guaranteed revert and push guard.
- **M4: Providers and sync.** Provider traits; Jira via `acli` (Review column, priority, @mentions, statuses), Bitbucket via `bkt` (PRs, pipelines by commit). Verify JSON, mention data and inline comments first; pin CLI versions. Cache, refresh, offline.
- **M5: Write gateway and integration.** The `uat` merge-and-push flow (conflict reporting, recorded merge commits), deploy-comment drafts, Alpha Testing transition, pipeline re-run, all confirmed and audited.
- **M6: Next-action engine.** Suggestion/Action model, the rules above, priority, dismiss/snooze persistence. Exposed headlessly as `de next` so rules can be tuned on real data before any UI.
- **M7: Menubar app.** GPUI shell: ticket views (review queue, active, parked), ticket detail, notifications, start/stop workspace. Needs a `.app` bundle; `cargo-dist` does not build one.
- **M8: In-app review.** Our own diff view (Zed's diff code is GPL-3, `de` is MIT), inline PR comments, later approve.
- **M9: After alpha.** UAT sign-off tracking, Returned handling, re-merge detection, PR approval suggestion.
- **Later:** GitHub adapter, time reports, reverting a ticket out of `uat`.

## Non-goals

- Autonomous external action: never a write without a confirmation, however confident the suggestion.
- Multiple simultaneously active tickets.
- Merging PRs through the host (this flow does not use it).
- Team features, shared config, multi-user sync, Windows/Linux.
- Per-ticket environments (env files, ports, seed data) beyond the overlay above.

## Open questions

Flow details:

1. **Jira mapping.** Exact status names (Review, Alpha Testing, UAT, Returned), board id, the priority values, and the JQL that gives the Review column.
2. **`uat` details.** Branch name per repo (is it always `uat`?), what happens if `uat` moved between fetch and push, and whether some repos deploy differently.
3. **Deployment step.** Its name per repo, and how a pipeline run is found for a commit via `bkt`.
4. **Overlay details.** Which repos provide and which consume packages (this fills the per-repo config above), and the rebuild tasks per repo. One mechanical risk remains: the symlink to the provider's checkout already resolves inside the container, so the overlay can be applied from the host; the previous `composer.lock` must be restorable exactly, and `vendor/` rebuilt, on revert.
5. **Base branch fallback.** Is it always `develop`? Should a repo on base be fast-forwarded automatically on activation, and what if its base has diverged or the tree is dirty (stash, as for ticket branches)?
6. **Local gate.** Varies by repo; the app offers to run a repo's tasks but does not enforce. Revisit if it becomes a source of mistakes.
7. **Claiming and ordering.** What "claim" means in the app (local flag only), and how manual order combines with Jira priority.
8. **Notes and checklist.** Free-form vs template checklist; where notes live; whether the checklist is per ticket only.
9. **Time tracking.** Automatic from active-ticket time, manual timers, or both.
10. **Notifications.** Which suggestions raise macOS notifications, and quiet hours.

Technical:

11. **CLI viability.** `acli` (JSON, pagination, mention data, speed) and `bkt` (Cloud vs Data Center, inline comment creation, pipeline lookup by commit, JSON stability). Verify by running them against real accounts.
12. **Menubar process model.** One app process owning the store and sync loop with the CLI reading the same SQLite (WAL), or a separate daemon. Launch-at-login and `.app` packaging.
13. **`gpui-kit`.** Confirm it builds on the pinned toolchain (1.92.0) and what its editor offers for diff display, before M7/M8.
14. **Suggestion tuning.** Needs a feedback loop (dismiss reasons) once the engine runs on real data.
15. **Workspace concept.** How much of the current workspace registry survives once tickets own the repo set.
16. **Release plumbing.** Installed `dist` (0.32.0) mismatches the pinned 0.30.3; the release workflow still lists old targets. README and `docs/` describe removed commands.

## Current state

- Workspace of two crates (`de-core`, `de`); `start`, `stop`, `compose`, `run`, `exec`, `task`, `workspace`, `config` work.
- Removed: shims, setup/snapshot, doctor/status, the old git commands, service tasks.
- Tasks run through the shell with exit-code passthrough.
- No domain store, git layer, providers, engine or GUI yet.
