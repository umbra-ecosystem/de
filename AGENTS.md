# AGENTS.md

## Project

Rust CLI (`de`, v0.6.1) for managing isolated Docker Compose dev environments. Edition 2024. Cargo workspace of three crates:

- `crates/core` (`de-core`, lib) — workspace/project config, task resolution, Compose orchestration. No CLI concerns.
- `crates/cli` (package `de`, the binary) — `clap` definitions, `src/commands/*`, terminal UI.
- `crates/widgets` (`de-widgets`, lib + `showcase` bin) — the GUI's view layer on `gpui-kit`: view models, widgets, and a fully working showcase app over a simulated store. **No engine, no data, no SQLite.** See "Widgets and the GUI" below.

Dependency versions live in the root `[workspace.dependencies]`; crates use `dep.workspace = true`.

## Build & test

- Toolchain is pinned to **1.92.0** by `rust-toolchain.toml`; use `cargo +1.92.0` or let the toolchain file select it.
- `cargo build` — builds the workspace. `cargo test --workspace` — runs the inline `#[cfg(test)]` tests across both crates (several hundred; most use real temporary git repos and SQLite files, so they need the `git` binary and take ~20s). There is no separate integration-test suite.
- `cargo clippy --workspace --all-targets` is clean; keep it that way.
- There is **no CI test/lint gate**; `.github/workflows/release.yml` only runs cargo-dist on version tags/PRs.

## Non-negotiable lints

Configured once in the root `[workspace.lints]` and inherited by each crate via `[lints] workspace = true`. Your code must not add violations:
- `clippy::implicit_clone` — do not call `.to_string()`/`.to_path_buf()`/`.to_owned()` on a `String`/`PathBuf`/`&str` you already own by value.
- `clippy::inconsistent_struct_constructor`
- `unused_must_use`


## Architecture

- Entrypoint: `crates/cli/src/main.rs` parses `cli.rs` `Cli`/`Commands` and dispatches to `crates/cli/src/commands/*` (one file per subcommand).
- Subcommand groups: `commands/{task,workspace}` are directories with a `mod.rs`; top-level commands are single files.
- `de-core` modules:
  - `workspace` (workspace config, active-workspace state, dependency-ordered `ordered_projects`), `project` (manifest incl. `[branches]`/`[overlay.composer]`/`[activate]`, `Project::compose`, task resolution/detection), `config`, `types`, `utils::get_project_dirs`.
  - `store` — two SQLite databases (`state.db` precious local data, `cache.db` disposable remote mirror), WAL, append-only migrations (state is at version 7, cache at 3; add an upgrade test with every migration). Free functions over `&Store`; time is always passed in (`now: i64`), never read inside.
  - `domain` — `TicketKey`, `LocalStatus` (+ transition table), `TicketKind`, `BaselineChoice`, link/audit enums.
  - `git` — reads via `git2` (`GitRepo`), mutations and network via the `git` CLI (`GitRunner`, so the user's SSH/credential/hooks config applies). Never force, never `reset --hard`.
  - `overlay` — the Composer test overlay (path/symlink repository, constraint `*`): apply, byte-exact revert from persisted backups, and the push guard `check_range_for_overlay`. External commands go through the `CommandRunner` trait so tests can fake them.
  - `activation` — `plan_activation` (pure), `activate`, `deactivate`/`park`. One active ticket at a time (enforced in SQL).
  - `providers` — the seam to Jira and Bitbucket: read traits (`TicketProvider`, `CodeHost`) and **separate write traits** (`TicketWriter`, `CodeHostWriter`), model types, `ProviderError`, in-memory `fake`s, and real adapters over the CLIs: `acli` (Jira) and `bkt` (Bitbucket, Cloud only). **Direction: GitHub (`gh`, incl. Actions runs) replaces `bkt`; see M10 in `GOAL.md`.** The code still has only the `bkt` adapter. Adapters never handle credentials; each CLI owns its login. Fixtures under `providers/fixtures/` must be **synthetic** (real structure, invented values): never commit real names, emails, hosts, keys or ticket text.
  - `sync` — read-only refresh of the cache from providers (`sync_jira`, `sync_code_host`, `sync_all`); a failure in one source never aborts others or deletes cached data. Also `derive_kind` (hotfix = an open PR targets the production branch).
  - `gateway` — **the only code that writes to Jira, Bitbucket or a remote git branch.** Typed `Action` -> `Draft` (exact preview) -> `Confirmed` (only via `Draft::confirm`, one use, payload-hash checked) -> `execute`, with an audit entry written *before* acting. No flag skips confirmation. Only the gateway may call `build_*_writer` (a test scans the sources).
  - `integration` — the `uat` flow: `prepare_integration` (temporary worktrees, merge, overlay guard on SHAs, pushes nothing), `execute_push` via the gateway, `finalize_integration` (the only way a ticket becomes `Integrated`), `deploy_status`, `needs_remerge`, deploy-comment drafts.
  - `next` — the next-action engine: pure `suggest(&Snapshot, now)`, `Snapshot::load` (impure, never calls providers), response filter (dismiss/snooze/done with a facts hash), local executors. `de next --json` is a **versioned contract** for the GUI (golden test).
  - `testsupport` (test-only) — fixture of real temp git repos with bare origins and a real `state.db`; tests here use real git and SQLite, not mocks.
- `crates/cli/src/commands/{ticket,git,sync,ship,next}.rs` are thin headless commands over those modules (`de ticket ...`, `de git status`, `de sync`, `de providers check|probe`, `de ticket integrate|deploy|draft-comment|post-comment|transition`, `de next`); keep rendering in pure functions separate from data gathering.
- The product direction and milestone status live in `GOAL.md`. Everything except the GUI (menubar app, in-app review) is built; the GUI must be a thin client over `de-core` and `de next --json`.
- Live tests that run the real `acli`/`bkt` read-only are `#[ignore]`d and named `live_*`: `cargo test -p de-core -- --ignored live_`. `de providers probe` prints raw tool output for diagnosing shape mismatches (it can contain ticket titles; review before sharing).
- Never run an `acli`/`bkt` **write** command outside the gateway flow, and never against real systems in tests.
- `crates/cli/src/main.rs` re-exports the core modules at its crate root, so CLI code writes `crate::project::…` for `de_core::project::…`.
- `PROJECT_NAME` is the literal `"de"` (not `CARGO_PKG_NAME`): it names the config dir and updater receipt, so it must not follow the crate name.
- Compose calls go through `Project::compose(args)` (`docker compose -f <file> …`); `de compose -- <args>` is the passthrough for everything that isn't `start`/`stop`.
- Config lives in the OS config dir (see `de_core::utils::get_project_dirs`) → `config.toml`, holding the active workspace. Per-directory state lives in a `.de/` dir (gitignored).
- `de` **dogfoods itself**: this repo is a `de` project (`.de/config.toml`, `de.toml` define tasks/setup). Don't commit anything under `.de/`.

## Widgets and the GUI

**All new UI is built in `crates/widgets` first, shown and exercised in the showcase, and only then used by the `de-app` crate.** Never write a screen or widget directly in `de-app`.

- `crates/widgets` depends on `gpui-kit` only. It must never depend on `de-core`, `rusqlite`, or any provider/git code, and never read the disk or network. Keep it that way: it is what lets the view be built and judged without the engine.
- Layers: `vm` (plain-data view models and the `Intent`/`Command` a view may emit; no GPUI) → `store::Store` (the seam) → `session::Session` (UI state: route, tabs, sheets, toasts, typed text; no GPUI, unit-tested) → `ui` (stateless GPUI widgets and screens that draw a view model and emit intents; the window is a `gpui-kit` `TitleBar` plus a `DockArea` whose panels just ask `AppView` to draw their part of the current frame).
- Views never hold state or call the engine. State lives in the session; data comes from a `Store`. The showcase's store is `sim::Sim` (an in-memory port of the prototype in `docs/design/prototype`); `de-app` will implement `Store` over `de-core` and `de next --json`.
- A remote write is a `Command` for which `Command::is_remote()` is true. It reaches the store only as a `Confirmed`, and only `Preview::confirm` makes one (it refuses a blocked tool and a missing typed key). `Store::dispatch` takes a `LocalCommand` only. Adding a `Command` variant forces a decision in the exhaustive test in `vm/intent.rs`; a remote one needs a literal preview in `Sim`/the real store.
- Model data with types, not strings: `TicketKey`, `RepoName`, `Branch`, `PrNumber`, `LineAnchor`, `HunkId` (in `vm/ids.rs`); enums for statuses, rules and outcomes; `Outcome` is an enum, never a bool plus optional fields. A ticket's local lifecycle is the single `Stage` enum in `sim/model.rs`; change it only through its transition methods (the table test there lists every legal move).
- Add a widget: view-model type in `vm`, a stateless function in `ui`, a case in the showcase. Add a flow: a `Session` test that drives it through intents only (`cargo test -p de-widgets`).
- **Always use the components `gpui-kit` ships before building your own** (`gpui_kit::component::*`): `Button`, `Checkbox`, `Input`/`Textarea`, `Tag`, `Avatar`, `Empty`, `TabBar`/`Tab`, `Sidebar`, `List`/`ListItem`, `Table`, `DockArea`, `TitleBar`, `Dialog`/`Sheet`, `Notification`, `Popover`, `Tooltip`, `Kbd`, `Switch`, `Stepper`, `DescriptionList`, `Command`. Check the crate (`~/.cargo/registry/src/*/gpui-component-*/src`) for a component before writing a div. Build from GPUI primitives only when nothing fits, in `ui/widgets.rs`, with a doc comment saying why. Long lists are `uniform_list` of fixed-height rows, not stacks of cards.
- **Use the built-in theme, nothing else for colour.** Read colours through `cx.theme()` (see `ui/theme.rs`); never write a hex or `hsla(..)` literal. Light, dark and system come from `Theme::change` / `Theme::sync_system_appearance`.
- **Bold is reserved for absolute attention**: something broke, someone is waiting, a hotfix, a high-risk confirmation (`SuggestionCard::attention`, bad-tone banners, rows with a failing flag). Everything else is regular weight; separate things with size, colour and position, never weight.
- **Ticket screens have a fixed head** (key, title, status tags, actions, tab bar) and a body that scrolls on its own, so the tabs never move. Review is the exception: it fills the body with panes (files, diff) between a toolbar and an action bar and each pane scrolls itself. Soft notices (overlap, new commits) are one quiet line; only a real conflict or blocker gets a box. A clean merge says nothing.
- **Hierarchy, on every screen** (the tab strip / header strip is the chrome; below it):
  1. Chrome says where you are (tab, nav item, ticket header with its status badges and stepper). **Never repeat it below**: no page title that equals the tab, no "Jira status" row next to the header's badge.
  2. At most one page title (`text_lg`), only when it adds information the tab lacks. Group labels are `text_xs` muted sentence case; content is `text_sm`; metadata is `text_xs` faint. Headings inside rich text are `text_base`, not bold.
  3. **Show exceptions, not defaults.** No pill for "up", "clean", "ready", "0". Silence is the normal state; a badge means something is off.
  4. Group with space and hairlines, never boxes or cards; sections in a side panel are separated by a hairline and a label.
  5. Side panels are summaries: what the main view does not already say, grouped (people, planning, tags, pull requests), at most about eight rows in view; detail goes down a step in brightness (title, then faint detail), not in weight.
  6. Hotspots get one loud thing (the bold title, the red alert), never several.
- **List screens filter the same way**: a 48px strip with the search field (`Inputs::search`: icon, capped at 360px, right-aligned) and one `ui/filter_menu.rs` dropdown (groups of checkable rows, a count of what is on, "Show everything"). Add a filter as a `FilterRow` and an `Intent`; do not add chips or toggles.
- **Icons**: the kit embeds only its 101 component icons (`IconName` in `gpui_kit::component`); an icon from the full Lucide catalog (`gpui_kit::assets::IconName`) draws nothing until its SVG is added to `ui/assets.rs` (`EXTRA`, files in `crates/widgets/assets/icons/`).
- Still hand-built and to be replaced by the kit's component when touched: navigation (`Sidebar`), toasts (`Notification`), sheets (`Dialog`), the palette (`Command`), the ticket stepper (`Stepper`), key/value rows (`DescriptionList`), and the audit/repo tables (`Table`).
- Look at the showcase without grabbing the whole screen: `DE_SHOWCASE_ROUTE` (`next`, `tickets`, `uat`, `workspace`, `audit`, `settings`, `ticket:PROJ-142:review`) and `DE_SHOWCASE_THEME` (`light`, `dark`) open a specific screen; capture only the app's window, never the full display.
- Run the showcase: `cargo run -p de-widgets --bin showcase` (it needs a desktop session; `gpui` opens a real window).

## Releases

- Release flow is cargo-dist (v0.30.3, see `dist-workspace.toml`), triggered by pushing a version tag. Targets are macOS only (`aarch64`/`x86_64-apple-darwin`).
- Bump `version` in the root `[workspace.package]` and add a `CHANGELOG.md` entry first, then tag:
  `just deploy 0.6.2` (= `git tag 0.6.2 && git push origin 0.6.2`).

## Docs

- `README.md` and `docs/` describe the product (user-facing, not internals). They are currently **stale**: they still document the removed commands (`logs`, `ps`, `down`, `restart`, `pull`, `build`, `doctor`, `status`, `git`, `setup`, `shim`) and service tasks.
