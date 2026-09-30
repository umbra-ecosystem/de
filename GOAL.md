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
2. **Review.** What is reviewed is **the PR's own diff: source branch against the PR's destination branch**, read from the PR, never assumed. PRs normally target `develop`, but **hotfix PRs target `master`/`main`**, and `uat` is never a PR target. Diffs come from git objects, not the working tree, so nothing is checked out and **any number of tickets can be under review at once**. Comments go on the Bitbucket PR (inline). **Approval is not given here**: the PR can only be approved once testing, including UAT after alpha, is complete. PRs are never merged through Bitbucket in this flow.
3. **Local test.** Exactly **one ticket is active** at a time (each repo can only be on one branch and the stack runs one version). Activating a ticket:
   - switches every repo the ticket touches to its branch, in place, stashing whatever was there. A ticket **touches only the repos that have a matching branch**; every **other repo goes to a baseline branch** and is brought up to date (`develop` by default, configurable per repo; for a **hotfix** you are asked which baseline to use, see below), so the whole stack is the ticket's code on a known baseline;
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
| Repos without the ticket branch | **Fall back to a baseline branch** (`develop` by default; **asked each time for hotfixes**), fetched and fast-forwarded; skipped for review and integration |
| PR target branch | **Read from each PR**, never assumed. Normally `develop`; hotfixes target `master`/`main`. `uat` is only ever pushed to directly, never a PR target. |
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

### Ticket kind: normal or hotfix

A ticket is a **hotfix** when its PRs target the production branch (`master` or `main`, whichever the repo uses) instead of `develop`. The app derives this from the synced PRs; it is never a manual flag (though it can be overridden). It matters in three places:

- **Review baseline:** always the PR's destination, so the diff is correct in both cases.
- **Fallback baseline** for repos the ticket does not touch: `develop` for a normal ticket. For a hotfix the app **asks each time** you activate it (production branch, `develop`, or `uat`), since the right answer depends on the fix. The choice is remembered for that activation only.
- **Suggestions:** hotfixes are flagged and prioritised. **After review a hotfix follows the same flow as any ticket**: merge to `uat`, push, alpha deploy, deploy comment, transition. Only the PR destination differs.

### Per-repo configuration

Repos differ, so behaviour is declared per project (in that project's `de.toml`, next to the existing `[project]` and `[tasks]`), not decided per ticket. Illustrative shape, to be settled in M1/M3:

```toml
[branches]
base       = "develop"   # fallback when the ticket has no branch here (normal tickets)
production = "master"    # "master" or "main" per repo; one of the baseline choices offered for hotfixes
uat        = "uat"       # integration branch (pushed to directly, never a PR target)

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
- Defaults come from the existing workspace `default_branch` setting where present, then `develop`. Review diffs do not use these; they use each PR's own destination.

### Safety: the test overlay must never reach `uat`

The `composer.json` change for local testing (the path repository and the `*` constraint) is deliberately temporary and must **never be committed or pushed**. It also changes **`composer.lock`** and the consumer's `vendor/` (the package becomes a symlink), so all three are part of the overlay. So:

- The app records exactly what the overlay changed (`composer.json` and `composer.lock` contents before and after) and reverts them on deactivate, then re-runs `composer install` so `vendor/` no longer points at the symlink.
- Integrating (merging and pushing `uat`) must not happen from a tree carrying the overlay. Do it in a **temporary worktree** so it also does not disturb the active test checkout, or refuse until the overlay is reverted.
- Before any push, a guard verifies that nothing from the overlay is in the commits being pushed.

## Integrations and CLI coverage

The first version of this table came from published docs. `acli` (1.3.39) is now installed and logged in on the author's machine and was **checked against its real output** (facts below marked *verified*); `bkt` (0.32.1) is installed but **not logged in**, so only its version and logged-out behaviour are verified. `gh` is not installed.

| System | Tool | Covers | Gaps / to verify |
|---|---|---|---|
| Jira | `acli jira workitem` (Atlassian's) | view, search, edit, assign, transition, comment create/list/update, link. *Verified:* `search --json` returns a JSON array of Jira-REST-like issues; `view --json` one issue. | *Verified:* **search rejects `--fields updated`** (`updated` is only allowed in `view`), and `--fields key` alone returns nulls. **@mentions: verified possible, but only via `view KEY --json --fields comment`**, which returns full comments (account id, ADF body with `mention` nodes, `created`); `comment list` returns flat strings with **no account id, timestamps or mentions** and must not be used for that. Unverified: `--paginate` output, logged-out wording, all writes, board/column and priority queries. |
| Bitbucket | `bkt` (community, [avivsinai/bitbucket-cli](https://github.com/avivsinai/bitbucket-cli), Go, MIT; **0.32.1 installed**) | PR list/view/create/merge/approve/request-changes, comment resolve/reopen/delete, pipelines list/view/run/rerun/logs, `--json`/`--yaml`, OS-keychain credentials, Cloud and Data Center | Read from source (not run): `bkt` can create inline comments (`pr comment --to-line/--from-line`) but prints no result, so the adapter posts through `bkt api`; it **cannot look up a pipeline by commit** (its pipeline JSON drops the commit and deployment environment), so the adapter lists runs through `bkt api` and filters, scanning at most 500. *Verified:* logged out, `auth status --json` is `{"hosts": null, "contexts": null}` and every other command fails with `no active context; run bkt context use <name>`. **The user must run `bkt auth login` and `bkt context use` once** before anything else here can be checked. Cloud only; Data Center returns `Unsupported`. |
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
- *Claimed, not reviewed:* suggest starting the review (open the PR's diff against its destination).
- *PR targets `master`/`main`:* flag as a hotfix and raise its priority; on activation, ask which baseline the untouched repos should use.
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
  - **Domain model**: Ticket (with a derived kind: normal or hotfix), Repo, Branch link, PR (source, **destination**, reviewers, approvals), Pipeline run, `uat` merge record, Overlay record, Checklist/Note, Time entry, Suggestion response.
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
- **M1: Done** (except importing the current workspace config, deferred). Domain and store. Ticket/repo/branch model, SQLite with migrations, local state (claims, ordering, notes, checklist, time), audit log. Import current workspace config.
- **M2: Done.** Git layer. Structured status, key matching with manual override, safe in-place switch with stash/restore, diff-from-objects, temporary worktrees, restore the uncommitted/unpushed guard on `stop`.
- **M3: Done**, independently reviewed and fixed. Active ticket and local test. Activate/park/restore, overlay engine (composer pointing at the ticket branch, rebuild tasks) with guaranteed revert and push guard.
- **M4: Done** (adapters unverified for writes and for Bitbucket reads). Providers and sync. Provider traits; Jira via `acli` (Review column, priority, @mentions, statuses), Bitbucket via `bkt` (PRs, pipelines by commit). Verify JSON, mention data and inline comments first; pin CLI versions. Cache, refresh, offline.
- **M5: Done.** Write gateway and integration. The `uat` merge-and-push flow (conflict reporting, recorded merge commits), deploy-comment drafts, Alpha Testing transition, pipeline re-run, all confirmed and audited.
- **M6: Done.** Next-action engine. Suggestion/Action model, the rules above, priority, dismiss/snooze persistence. Exposed headlessly as `de next` so rules can be tuned on real data before any UI.
- **M7: Menubar app.** GPUI shell: ticket views (review queue, active, parked), ticket detail, notifications, start/stop workspace. Needs a `.app` bundle; `cargo-dist` does not build one.
- **M8: In-app review.** Our own diff view (Zed's diff code is GPL-3, `de` is MIT), inline PR comments, later approve.
- **M9: Done** (as engine rules). After alpha. UAT sign-off tracking, Returned handling, re-merge detection, PR approval suggestion.
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
6. **Hotfix details.** Confirm `master` vs `main` per repo, and whether hotfixes need any different priority or notification handling than normal tickets.
7. **Local gate.** Varies by repo; the app offers to run a repo's tasks but does not enforce. Revisit if it becomes a source of mistakes.
8. **Claiming and ordering.** What "claim" means in the app (local flag only), and how manual order combines with Jira priority.
9. **Notes and checklist.** Free-form vs template checklist; where notes live; whether the checklist is per ticket only.
10. **Time tracking.** Automatic from active-ticket time, manual timers, or both.
11. **Notifications.** Which suggestions raise macOS notifications, and quiet hours.

Technical:

12. **CLI viability.** *Resolved for acli reads:* JSON shapes and @mention data (via `view --fields comment`) are real and usable. *Still open:* acli `--paginate` output and its writes (comment create, transition; the CLI cannot list transitions), and everything about `bkt` beyond its logged-out behaviour: Cloud vs Data Center, the shapes of `pr view`/`pipeline` output through `bkt api`, the deployment-environment field on pipeline steps, inline comment creation. Verify by logging in and running `de providers probe --repo workspace/slug`, then `cargo test -p de-core -- --ignored live_`.
13. **Menubar process model.** One app process owning the store and sync loop with the CLI reading the same SQLite (WAL), or a separate daemon. Launch-at-login and `.app` packaging.
14. **`gpui-kit`.** Confirm it builds on the pinned toolchain (1.92.0) and what its editor offers for diff display, before M7/M8.
15. **Suggestion tuning.** Needs a feedback loop (dismiss reasons) once the engine runs on real data.
16. **Workspace concept.** How much of the current workspace registry survives once tickets own the repo set.
17. **Release plumbing.** Installed `dist` (0.32.0) mismatches the pinned 0.30.3; the release workflow still lists old targets. README and `docs/` describe removed commands.

## Current state

Everything before the GUI is built and tested (about 480 tests, real temporary git repos and SQLite; provider fakes for Jira and Bitbucket), and has had independent adversarial review of the overlay/activation code and, in progress, of the gateway/push path and the engine/sync/adapters.

- **Built:** store and domain (M1), git layer, `de git status`, `de stop` guard (M2), activation and the Composer overlay with a fail-closed push guard (M3), provider contracts, sync engine, `acli` and `bkt` adapters (M4), write gateway, `uat` integration, deploy tracking and comment drafts (M5), next-action engine and after-alpha rules with `de next --json` (M6, M9).
- **Verified against real tools:** `acli` 1.3.39 reads (search, view, comments and @mentions via `view --fields comment`), via opt-in `live_` tests. `bkt` 0.32.1 only up to its logged-out behaviour.
- **Not verified:** any write to Jira or Bitbucket (comment, transition, PR comment, approve, pipeline trigger), `bkt` reads (needs `bkt auth login` and `bkt context use`), `git push` against hosted Bitbucket (only local bare remotes), Composer overlay against real Composer, interactive `de next do`.
- **Pending: the GUI.** M7 (menubar app: ticket views, notifications, background sync loop, start/stop workspace) and M8 (in-app review: own diff view, inline PR comments, later approve). The GUI must render `de next --json` and call `de-core`, with no business logic of its own. Spike `gpui-kit` first (open question 13).
- **Known gaps:** see the review notes above and each milestone's listed gaps; hotfix kind is derived from synced PRs but the ticket CLI still shows the manual override; README and `docs/` describe removed commands.
- Removed earlier: shims, setup/snapshot, doctor/status, the old git commands, service tasks.
