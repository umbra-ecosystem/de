# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- Fuzzy branch selection and `de git switch` command for managing branches across all workspace projects.
- Ability to set and unset arbitrary workspace properties (such as `default-branch`) via `de workspace set` and `de workspace unset`.
- Support for querying the value of a workspace property with `de workspace set <key>`.
- Improved CLI documentation and examples for workspace management.

### Changed
- Refactored workspace property management to use key/value pairs, making it extensible for future properties.
- Improved output and error handling for workspace commands.

### Removed
- Unused imports and minor internal code cleanups.

---

## [0.2.1] - 2025-07-06

### Added
- `de workspace info` command for detailed workspace information.
- `de workspace set` and `de workspace unset` commands for managing the active workspace.
- Ability to specify project and workspace options for the `run` command, improving flexibility and error output.

### Changed
- The `list` command now uses the active workspace by default for more intuitive behavior.

---

## [0.2.0] - 2025-07-04

### Added
- Workspace-level task support and related CLI commands.
- `de exec-all` command to run commands in all projects of a workspace.
- `de exec` command to run arbitrary commands in a project context.
- Fallthrough command to handle unknown subcommands gracefully.
- Project and workspace selection for task add/remove commands.
- Console-based formatting and theming utilities for improved CLI output.
- Enhanced error handling with color-eyre.
- Documentation for doctor and status commands.
- Tracing logs and improved status summaries for better observability.
- `de status` and `de doctor` commands for workspace and project health checks.
- Uninstall command for shims and improved install logic.
- Workspace status check and prompt in the stop command.
- Warnings for task name conflicts with project names in doctor command.

### Changed
- Refactored doctor command for improved formatting and clarity.
- Formatter methods now return `Result` and propagate errors.
- Updated to use modern Rust format string syntax.
- Task command parsing uses split_whitespace for better reliability.
- Doctor command now outputs structured diagnostics.
- Indented doctor output for readability.
- Expanded and revised feature list and documentation in README.
- Moved shim module to utils and updated imports.
- Compose tasks are now validated against available services.

### Removed
- Dropped support for aarch64-pc-windows-msvc build target.
- Removed unsafe block for setting RUST_BACKTRACE.

---

## [0.1.0] - 2025-06-25

### Added
- Initial project structure and implementation of `de init`.
- Project tasks and `de run` command.
- `de list` command to show projects in a workspace.
- Task and shim management commands.
- `de start` and `de stop` commands for workspace project management.
- `de scan` command for project discovery.
- Docker Compose management instructions in README.
- Justfile with deploy recipe for tagging and pushing.
- Self-update command using axoupdater and smol.
- Recursive project discovery.
- Workspace synchronization and update command.
- Dialoguer and tracing dependencies for interactive CLI.
- Optional project path and name in init command.
- Package description in Cargo.toml.
- GitHub Actions release workflow and dist config.
- Task list command to show all tasks in a project.
- dotenvy dependency and .env loading in project initialization.

### Changed
- Refactored workspace and manifest config structure.
- Improved manifest handling and project naming logic.
- Project config now uses Slug as project ID.
- Project loading now uses directory instead of manifest path.
- Scan command accepts directory as positional argument.
- Project loading simplified by removing manifest existence check.
- Used project name instead of inferred ID when updating workspace.
- Workspace name moved from top-level to project metadata.
- Improved and expanded README documentation and examples.
- Improved generated bash shim script and clarified comments.
- Fixed docker_compose_path to resolve relative paths correctly.
- Config file loading now uses TOML format and correct paths.
- Set environment variable separator for config loading.
- Task commands now run in the project directory.
- Task commands now support passing arguments and improved error handling.
- Removed redundant note about setup on new machines.
- Removed Windows target from build configuration.

### Removed
- Removed smol and switched updater to blocking mode.
- Removed debug print statements.

---

[Unreleased]: https://github.com/umbra-ecosystem/de/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/umbra-ecosystem/de/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/umbra-ecosystem/de/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/umbra-ecosystem/de/releases/tag/v0.1.0
