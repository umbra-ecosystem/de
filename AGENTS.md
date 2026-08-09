# AGENTS.md

## Project

Rust CLI (`de`, v0.6.1) for managing isolated Docker Compose dev environments. Edition 2024. Single binary crate — not a workspace.

## Build & test

- Toolchain is pinned to **1.92.0** by `rust-toolchain.toml`; use `cargo +1.92.0` or let the toolchain file select it.
- `cargo build` — builds. `cargo test` — runs the only tests, which are inline `#[cfg(test)]` unit tests in `src/workspace/dependency.rs` and `src/project/task_detector.rs`. There is no test harness or integration-test suite.
- `cargo clippy` **currently errors** (4 pre-existing `clippy::implicit_clone` violations). This is a known state, not a regression.
- There is **no CI test/lint gate**; `.github/workflows/release.yml` only runs cargo-dist on version tags/PRs.

## Non-negotiable lints

`#![deny(...)]` at the top of `src/main.rs` applies crate-wide. Your code must not add new violations:
- `clippy::implicit_clone` — do not call `.to_string()`/`.to_path_buf()`/`.to_owned()` on a `String`/`PathBuf`/`&str` you already own by value.
- `clippy::inconsistent_struct_constructor`
- `unused_must_use`

Keep clippy as clean as you can; don't make the error count worse.

## Architecture

- Entrypoint: `src/main.rs` parses `src/cli.rs` `Cli`/`Commands` and dispatches to `src/commands/*` (one file per subcommand).
- Subcommand groups: `src/commands/{git,task,workspace,shim}` are directories with a `mod.rs`; top-level commands are single files.
- Key modules: `src/workspace` (workspace config, active-workspace state), `src/project` (project config, task detection), `src/setup` (snapshot/apply), `src/utils` (UI/theme, project dirs).
- Config lives in the OS config dir (see `src/utils/get_project_dirs`) → `config.toml`, holding the active workspace. Per-directory state lives in a `.de/` dir (gitignored).
- `de` **dogfoods itself**: this repo is a `de` project (`.de/config.toml`, `de.toml` define tasks/setup). Don't commit anything under `.de/`.

## Releases

- Release flow is cargo-dist (v0.30.3, see `dist-workspace.toml`), triggered by pushing a version tag.
- Bump `Cargo.toml` version and add a `CHANGELOG.md` entry first, then tag:
  `just deploy 0.6.2` (= `git tag 0.6.2 && git push origin 0.6.2`).
- `Shim` subcommands are gated with `#[cfg(target_family = "unix")]`; keep platform-gated code consistent.

## Docs

- `CLI_COMMANDS.md` and `CORE_FEATURES.md` are generated/kept-in-sync with the CLI; update them when command surface changes. `README.md` and `docs/` describe the product (user-facing, not internals).
