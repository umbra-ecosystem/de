# AGENTS.md

## Project

Rust CLI (`de`, v0.6.1) for managing isolated Docker Compose dev environments. Edition 2024. Cargo workspace of two crates:

- `crates/core` (`de-core`, lib) — workspace/project config, task resolution, Compose orchestration. No CLI concerns.
- `crates/cli` (package `de`, the binary) — `clap` definitions, `src/commands/*`, terminal UI.

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

## Releases

- Release flow is cargo-dist (v0.30.3, see `dist-workspace.toml`), triggered by pushing a version tag. Targets are macOS only (`aarch64`/`x86_64-apple-darwin`).
- Bump `version` in the root `[workspace.package]` and add a `CHANGELOG.md` entry first, then tag:
  `just deploy 0.6.2` (= `git tag 0.6.2 && git push origin 0.6.2`).

## Docs

- `README.md` and `docs/` describe the product (user-facing, not internals). They are currently **stale**: they still document the removed commands (`logs`, `ps`, `down`, `restart`, `pull`, `build`, `doctor`, `status`, `git`, `setup`, `shim`) and service tasks.
