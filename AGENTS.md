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
  - `store` — two SQLite databases (`state.db` precious local data, `cache.db` disposable remote mirror), WAL, append-only migrations (state is at version 3; add an upgrade test with every migration). Free functions over `&Store`; time is always passed in (`now: i64`), never read inside.
  - `domain` — `TicketKey`, `LocalStatus` (+ transition table), `TicketKind`, `BaselineChoice`, link/audit enums.
  - `git` — reads via `git2` (`GitRepo`), mutations and network via the `git` CLI (`GitRunner`, so the user's SSH/credential/hooks config applies). Never force, never `reset --hard`.
  - `overlay` — the Composer test overlay (path/symlink repository, constraint `*`): apply, byte-exact revert from persisted backups, and the push guard `check_range_for_overlay`. External commands go through the `CommandRunner` trait so tests can fake them.
  - `activation` — `plan_activation` (pure), `activate`, `deactivate`/`park`. One active ticket at a time (enforced in SQL).
  - `testsupport` (test-only) — fixture of real temp git repos with bare origins and a real `state.db`; tests here use real git and SQLite, not mocks.
- `crates/cli/src/commands/{ticket,git}.rs` are thin headless commands over those modules (`de ticket ...`, `de git status`); keep rendering in pure functions separate from data gathering.
- The product direction and milestone status live in `GOAL.md`. Integrations (`acli`, `bkt`) and the GUI are not built yet.
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
