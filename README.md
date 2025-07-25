# de

[![Release](https://github.com/umbra-ecosystem/de/actions/workflows/release.yml/badge.svg)](https://github.com/umbra-ecosystem/de/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

`de` is a simple yet powerful CLI tool for managing isolated development environments with Docker Compose. It helps organize projects into workspaces and provides a unified interface for running tasks across different development environments.

## Features

- **Workspace & Project Management**
  - 🏗️ Initialize and organize projects with declarative configuration
  - 🌐 Group related projects into named workspaces
  - ✨ Manage workspace properties with `de workspace config <key> <value>`
  - ℹ️ Get detailed information about workspaces with `de workspace info`
  - 🔍 Automatically discover and register projects
  - 🔄 Synchronize and update workspace/project registrations

- **Task & Command Execution**
  - ⚡ Define and run both shell commands and Docker Compose service tasks
  - 🚀 Execute arbitrary commands within a project's environment
  - 🏃🏽‍♀️ Fallthrough for direct task execution without the `run` subcommand
  - 🔗 Create command shims/aliases for easy access
  - ✅ Check and list available tasks

- **Docker Compose Integration**
  - 🚀 Start and stop Docker Compose projects - current project + dependencies or entire workspace
  - 📦 Manage services across multiple projects
  - 🧩 Manage project dependencies with `depends_on` in `de.toml`

- **Environment & Configuration**
  - 🔧 Load `.env` files and environment variable configuration
  - 🏷️ Support for local overrides and configuration hierarchy

- **Diagnostics & Status**
  - 🩺 Diagnose environment, dependencies, and configuration with `de doctor`
  - 📊 Get a concise, actionable summary of Git and Docker Compose status with `de status`
  - 🧹 Easily reset all workspace projects to the base branch with `de git base-reset`

## Installation

### Pre-built Binaries

Download the latest release from the [GitHub releases page](https://github.com/umbra-ecosystem/de/releases).

### From Source

```bash
cargo install --git https://github.com/umbra-ecosystem/de
```

### Updating

If you installed `de` from a pre-built binary, you can update to the latest version using:

```bash
de self update
```

This will automatically download and install the latest release from GitHub.

## Quick Start

### 1. Initialize a Project

Create a new project and add it to a workspace:

```bash
cd my-project
de init
```

This will interactively prompt you for:
- **Workspace name**: Which workspace to add the project to
- **Project name**: The name for your project (defaults to directory name)

You can also specify these options directly:

```bash
# Initialize with specific workspace and project name
de init --workspace my-workspace --name my-api

# Initialize a specific directory
de init /path/to/project --workspace my-workspace
```

This creates a `de.toml` configuration file in your project directory.

### 2. Define Tasks

Edit the generated `de.toml` file to define tasks:

```toml
[project]
name = "my-api"
workspace = "my-workspace"
docker_compose = "docker-compose.yml"

[tasks]
# Raw shell command
test = "cargo test"

# Docker Compose service command
dev = { service = "api", command = "cargo watch -x run" }

# Complex task with explicit command
build = { command = "docker build -t my-api ." }
```

### 3. Run Tasks

Execute defined tasks from anywhere in your project. If a task is not found in the project, `de` will attempt to run a workspace task with the same name.

```bash
de run test
de run dev
de run build --release  # Pass additional arguments
de run --project my-api test # Run a task in a specific project
de run --workspace my-workspace build-all # Run a task in a specific workspace
```

### 4. Command Fallthrough (Direct Task Execution)

`de` supports direct task execution without the `run` subcommand for a more streamlined workflow. If a command is not a built-in `de` command, it will be treated as a task to be executed.

The fallthrough logic works as follows:

1.  **Workspace Project Task**: If the command matches a project name in the current workspace, `de` will attempt to execute the subsequent argument as a task within that project.
2.  **Current Project Task**: If the command is not a project name, `de` will attempt to execute it as a task in the current project.

This allows for a more natural and concise way to run your common tasks.

#### Examples

Given the following project `de.toml`:

```toml
[project]
name = "my-api"
workspace = "my-workspace"

[tasks]
test = "cargo test"
start = { service = "api", command = "cargo watch -x run" }
```

Instead of `de run test`, you can simply run:

```bash
de test
```

Instead of `de run start`, you can run:

```bash
de start
```

If you have another project named `my-frontend` in the same workspace, you could run its tasks like this:

```bash
de my-frontend build
```
This would execute the `build` task within the `my-frontend` project.

### 5. Define Workspace Tasks

Edit the `de.toml` file in your workspace configuration directory (usually `~/.config/de/workspaces/<workspace-name>.toml`) to define workspace-level tasks:

```toml
[project]
# ... existing project configurations ...

[tasks]
# Workspace-level tasks
build-all = "cargo build --all"
clean-all = "cargo clean --all"
```

### 6. Run Workspace Tasks

Execute defined workspace tasks:

```bash
de workspace run build-all
de workspace run clean-all --release
```

This allows you to run commands that apply to the entire workspace, such as building all projects or cleaning all build artifacts.

### 7. Execute Arbitrary Commands

Run any command within the context of a project's environment:

```bash
de exec <project> -- <args>
```

For example, to run `npm install` in a project:

```bash
de exec web -- npm install
```

Or to run a specific script with arguments:

```bash
de exec app1 -- python my_script.py --some-arg value
```

This command is useful for one-off operations or when you need to interact directly with the project's environment without defining a specific task in `de.toml`.

### 8. Execute Arbitrary Commands Across All Projects

Run any command within the context of all projects in a workspace:

```bash
de exec-all -- <command> <args>
```

For example, to run `npm install` in all projects in the current workspace:

```bash
de exec-all -- npm install
```

Or to run a specific script with arguments in all projects:

```bash
de exec-all -- python my_script.py -- --some-arg value
```

This command is useful for performing bulk operations across multiple projects in a workspace.

### 9. Reset All Projects to Base Branch

Reset all projects in your workspace to the base branch (e.g., `dev` or your configured default):

```bash
de git base-reset
```

By default, this command uses the workspace's `default_branch` property for branch operations. If no branch is specified, the configured default branch is used.

You can skip certain projects from all git commands (including base-reset, switch, and status) by setting `git.enabled = false` in the project's manifest (`de.toml`). This is useful for excluding projects that do not use git or should not be affected by workspace-wide git operations.

This command will:
- Fetch the latest changes from remotes for each project
- Detect and prompt for uncommitted changes (with options to stash, force reset, skip, or abort)
- Check out the base branch and hard reset to the remote version
- Clean untracked files

You can use this to quickly prepare your workspace for a new feature branch or to ensure all projects are in sync with the base branch.

### 10. Switch Branches Across All Projects

Switch all workspace projects to a specified branch, with fuzzy branch selection and interactive handling of dirty states:

```bash
de git switch [branch-name]
```

- If you provide a branch name, `de` will attempt to switch all projects to that branch.
- If the branch name is ambiguous or partially matches multiple branches, you'll be prompted to select the correct branch interactively.
- If a project has uncommitted changes, you can choose to stash, force checkout, skip, or abort for each project.
- If a project does not have the branch, the workspace's `default_branch` will be used as a fallback.

You can skip certain projects from this command by setting `git.enabled = false` in their manifest.

**Example:**
```bash
de git switch feature/login
```

This will:
- Fuzzy-match and select the correct branch across all projects
- Stash or force checkout dirty projects as needed
- Use the workspace's default branch as a fallback if the branch does not exist in a project

### 11. Switch Branches Across All Projects

Switch all workspace projects to a specified branch, with fuzzy branch selection and interactive handling of dirty states:

```bash
de git switch [branch-name]
```

- If you provide a branch name, `de` will attempt to switch all projects to that branch.
- If the branch name is ambiguous or partially matches multiple branches, you'll be prompted to select the correct branch interactively.
- If a project has uncommitted changes, you can choose to stash, force checkout, skip, or abort for each project.
- If a project does not have the branch, the workspace's `default_branch` will be used as a fallback.

You can skip certain projects from this command by setting `git.enabled = false` in their manifest.

**Example:**

```bash
de git switch feature/login
```

This will:
- Fuzzy-match and select the correct branch across all projects
- Stash or force checkout dirty projects as needed
- Use the workspace's default branch as a fallback if the branch does not exist in a project

---

### 12. Start/Stop Docker Compose Projects

Start Docker Compose projects:

```bash
# Start the current project and its dependencies
de start

# Start all projets in active workspace
de start --workspace

# Start all projects in a specific workspace
de start --workspace my-workspace
```

Stop all Docker Compose projects in a workspace:

```bash
de stop
de stop --workspace my-workspace
```

### 13. List Projects

View all projects in your current workspace (or the active workspace if set):

```bash
de list
```

Or list projects in a specific workspace:

```bash
de list --workspace my-workspace
```

### 14. Check Project and Environment Health

Diagnose and check the health of your `de` environment:

```bash
de doctor
```

This command will check for common issues, missing files, and misconfigurations in your workspace and projects.

### 15. Show Workspace and Project Status

Get a concise, actionable summary of the dynamic state of all projects in the current workspace:

```bash
de status
```

This command shows:
- Git status (uncommitted changes, ahead/behind remote)
- Docker Compose service status (up/down)
- A summary of actionable items with suggestions

## Project Initialization

### Initialize a New Project

Create a new project and add it to a workspace:

```bash
cd my-project
de init
```

The command will interactively prompt you for the workspace and project name if not provided. You can also specify them directly:

```bash
# With explicit options
de init --workspace my-workspace --name my-api

# Initialize a different directory
de init /path/to/project --workspace production
```

This creates a `de.toml` configuration file in your project directory with the following structure:

```toml
[project]
name = "my-api"
workspace = "my-workspace"
```

You can then add project metadata and tasks:

```toml
[project]
name = "my-api"
workspace = "my-workspace"
docker_compose = "docker-compose.yml"

[tasks]
test = "cargo test"
dev = { service = "api", command = "cargo watch -x run" }
```

### Configuration

Projects are configured using `de.toml` files with the following structure:

```toml
[project]
name = "project-name"
workspace = "workspace-name"
docker_compose = "docker-compose.yml"    # Optional: path to docker-compose file
depends_on = ["database", "cache"]       # Optional: projects that must start before this one

[tasks]
# Simple shell command
task-name = "command to run"

# Docker Compose service task
service-task = { service = "service-name", command = "command in service" }

# Complex shell command
complex-task = { command = "multi part command with args" }
```

#### Project Dependencies

The `depends_on` field allows you to specify which projects must be started before the current project. When using `de start` without a workspace parameter, it will start the current project and its dependencies in the correct order. When using `de start --workspace`, it starts all projects in the workspace, respecting all dependency relationships. This is particularly useful for microservices architectures where services have startup dependencies.

```toml
[project]
name = "web-app"
workspace = "my-workspace"
depends_on = ["database", "redis", "api"]
```

**Features:**
- **Startup Order**: Projects are started in dependency order (dependencies first)
- **Shutdown Order**: Projects are stopped in reverse dependency order (dependents first)
- **Cycle Detection**: Circular dependencies are detected and will cause an error
- **Validation**: Missing dependencies are detected and reported
- **Topological Sorting**: Uses Kahn's algorithm for efficient dependency resolution

**Example dependency chain:**
```
database → api → web-app
   ↓        ↓       ↓
  (1)      (2)     (3)
```

#### Task Types

**Raw Tasks**: Execute shell commands in the project directory
```toml
[tasks]
test = "npm test"
build = { command = "npm run build" }
```

**Docker Compose Tasks**: Execute commands inside Docker Compose services
```toml
[tasks]
dev = { service = "web", command = "npm run dev" }
shell = { service = "api", command = "bash" }
```

#### Environment Variables

- Load environment variables from `.env` files in your project directory
- Override configuration with `DE_` prefixed environment variables
- Use `.de/config.toml` for local project-specific overrides

#### Configuration Hierarchy

Configuration is loaded in the following order (later sources override earlier ones):

1. `de.toml` - Main project configuration
2. `.de/config.toml` - Local overrides (optional)
3. Environment variables with `DE_` prefix

### Advanced Usage

#### Workspace Management

Projects are automatically organized into workspaces. You can:

- Have multiple workspaces for different types of projects
- List projects across all workspaces
- Scan directories to auto-discover and register projects

#### Project Discovery

Automatically discover and register projects:

```bash
# Scan current directory for projects
de scan

# Scan specific directory
de scan ~/projects

# Scan for only specific workspace projects
de scan ~/production-apps --workspace production
```

#### Doctor

Diagnose your environment, dependencies, and project/workspace configuration:

```bash
de doctor
```

- Checks for required system dependencies (Docker, Docker Compose)
- Validates project and workspace configuration
- Reports missing files, misconfigurations, and actionable suggestions
- Checks if Docker Compose services referenced in tasks exist
- Warns if a task name conflicts with a project name in the same workspace

#### Status

Show a concise, actionable summary of the current workspace:

```bash
de status
```

- Shows Git status (uncommitted changes, ahead/behind remote)
- Shows Docker Compose service status (up/down)
- Summarizes actionable items with suggestions

#### Workspace Synchronization

Keep workspace configurations synchronized with your projects:

```bash
# Update current project's workspace registration
de update

# Update all projects in the current workspace
de update --workspace

# Update all projects in a specific workspace
de update --workspace my-workspace

# Update all workspaces and projects
de update --all
```

The `update` command helps maintain workspace integrity by:

- **Validating project existence**: Removes stale project entries for directories that no longer exist
- **Detecting name changes**: Updates registrations when project names change in `de.toml` files
- **Handling workspace migrations**: Removes projects that have moved to different workspaces
- **Refreshing configurations**: Ensures workspace registrations reflect current project states

Use `update` when workspace configurations need to be synchronized:
- After moving or deleting project directories
- After renaming projects in their `de.toml` files
- After migrating projects between workspaces

#### Workspace Commands

Manage your workspaces and workspace properties.

```bash
# Set a workspace property (e.g., default branch)
de workspace config default-branch main

# Get a workspace property
de workspace config default-branch

# Unset a workspace property
de workspace config default-branch --unset

# Set/get/unset properties for a specific workspace
de workspace config --workspace my-workspace default-branch main
de workspace config --workspace my-workspace default-branch
de workspace config --workspace my-workspace default-branch --unset

# Get information about the active workspace
de workspace info

# Get information about a specific workspace
de workspace info my-workspace
```

#### Self-Update

Keep `de` up to date with the latest features and bug fixes:

```bash
# Update to the latest version
de self update
```

The update command will:
- Check for the latest release on GitHub
- Download and install the new version if available
- Display the new version number after successful update
- Show "No updates available" if you're already on the latest version

#### Task Management

Manage tasks defined in your project's `de.toml` file and your workspace configuration.

```bash
# List all available tasks (from project and workspace)
de task list

# Check if a specific task is defined
de task check <task-name>

# Add a new task to the current project (raw command)
de task add my-task "echo Hello from project!"

# Add a new task to the current project (Docker Compose service command)
de task add my-service-task "npm run dev" --service web

# Add a new task to the active workspace
de task add --workspace my-workspace-task "echo Hello from workspace!"

# Remove a task from the current project
de task remove my-task

# Remove a task from the active workspace
de task remove --workspace my-workspace-task
```

#### Command Shims

Create command aliases that work from anywhere:

```bash
# Install shims support
de shim install

# Create a shim for a task (requires a task named 'php' in your de.toml)
de shim add php

# Now you can run 'php' from anywhere in your project
php
```

**Note**: Shims require a corresponding task with the same name defined in your project's `de.toml` file.

#### Reinstate Shims

To update or recreate all shims for your project tasks (for example, if shims are out of date), use:

```bash
de shim reinstate
```

This command will scan for all tasks with shims and rewrite all shim files, ensuring your command aliases are up to date and consistent with your current configuration.

#### Docker Compose Management

Start and stop Docker Compose projects across workspaces:

```bash
# Start the current project and its dependencies
de start

# Start all Docker Compose projects in a specific workspace
de start --workspace production
```

The `start` command will also set the workspace as the active one. When no workspace is specified, it starts only the current project and its dependencies in the correct dependency order. When a workspace is specified, it starts all projects in that workspace.

```bash
# Stop all Docker Compose projects in the active workspace
de stop

# Stop all Docker Compose projects in a specific workspace
de stop --workspace production
```

The `stop` command will check for uncommitted or unpushed changes and prompt for confirmation before stopping. It will also deactivate the workspace if it was the active one.

These commands automatically run `docker-compose up -d` and `docker-compose down` respectively for the relevant projects that have Docker Compose files configured. The `start` command without a workspace parameter will only start the current project and its dependencies, while specifying a workspace will start all projects in that workspace.

## Examples

### Web Application Project

```toml
[project]
name = "my-blog"
workspace = "web-apps"
docker_compose = "docker-compose.dev.yml"

[tasks]
# Development tasks
dev = { service = "web", command = "npm run dev" }
test = { service = "web", command = "npm test" }
lint = { service = "web", command = "npm run lint" }

# Database tasks
db-migrate = { service = "db", command = "npm run migrate" }
db-seed = { service = "db", command = "npm run seed" }

# Build tasks
build = "docker build -t my-blog ."
deploy = "docker push my-blog:latest"
```

### Microservices Workspace with Dependencies

Here's an example of a microservices workspace that demonstrates the `depends_on` feature:

**Database Project (no dependencies):**
```toml
[project]
name = "database"
workspace = "microservices"
docker_compose = "docker-compose.yml"

[tasks]
migrate = { service = "db", command = "migrate up" }
seed = { service = "db", command = "seed --env dev" }
```

**Cache Project (no dependencies):**
```toml
[project]
name = "cache"
workspace = "microservices"
docker_compose = "docker-compose.yml"

[tasks]
flush = { service = "redis", command = "redis-cli flushall" }
```

**API Service (depends on database and cache):**
```toml
[project]
name = "api"
workspace = "microservices"
docker_compose = "docker-compose.yml"
depends_on = ["database", "cache"]

[tasks]
dev = { service = "api", command = "cargo watch -x run" }
test = { service = "api", command = "cargo test" }
```

**Web Frontend (depends on API):**
```toml
[project]
name = "web"
workspace = "microservices"
docker_compose = "docker-compose.yml"
depends_on = ["api"]

[tasks]
dev = { service = "web", command = "npm run dev" }
build = { service = "web", command = "npm run build" }
```

**Startup Order:**
When you run `de start` from the `web` project directory, it will start the current project and its dependencies in this order:
1. `database` and `cache` (can start in parallel)
2. `api` (waits for database and cache)
3. `web` (waits for api)

If you run `de start` from the `api` project directory, it will only start:
1. `database` and `cache` (can start in parallel)
2. `api` (waits for database and cache)

To start all projects in the workspace regardless of your current directory, use:
```bash
de start --workspace microservices
```

**Shutdown Order:**
When you run `de stop`, the projects will stop in reverse order:
1. `web` (stops first)
2. `api` (stops after web)
3. `database` and `cache` (stop last, can be parallel)
```

## Contributing

We welcome contributions! Please see our [contributing guidelines](CONTRIBUTING.md) for details.

### Development Setup

1. Clone the repository:
   ```bash
   git clone https://github.com/umbra-ecosystem/de.git
   cd de
   ```

2. Install Rust and build:
   ```bash
   cargo build
   ```

3. Run tests:
   ```bash
   cargo test
   ```

### Release Process

Releases are automated using [cargo-dist](https://github.com/astral-sh/cargo-dist). To create a new release:

1. Update the version in `Cargo.toml`
2. Create and push a git tag: `git tag v0.1.0 && git push origin v0.1.0`
3. GitHub Actions will automatically build and publish the release

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Support

- 🐛 **Bug Reports**: [GitHub Issues](https://github.com/umbra-ecosystem/de/issues)
- 💡 **Feature Requests**: [GitHub Discussions](https://github.com/umbra-ecosystem/de/discussions)
- 📖 **Documentation**: This README and inline help (`de --help`)

---

Made with ❤️ by the [Umbra Ecosystem](https://github.com/umbra-ecosystem) team.
