# de

[![Release](https://github.com/umbra-ecosystem/de/actions/workflows/release.yml/badge.svg)](https://github.com/umbra-ecosystem/de/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

`de` is a simple yet powerful CLI tool for managing isolated development environments with Docker Compose. It helps organize projects into workspaces and provides a unified interface for running tasks across different development environments.

## Features

- 🏗️ **Project Management**: Initialize and organize projects with declarative configuration
- 🌐 **Workspace Organization**: Group related projects into named workspaces
- ⚡ **Task Execution**: Define and run both shell commands and Docker Compose service tasks
- 🔗 **Command Shims**: Create command aliases that can be added to your PATH
- 🔍 **Project Discovery**: Automatically scan and register projects in your workspaces
- 🚀 **Docker Compose Management**: Start and stop all Docker Compose projects in a workspace
- 🔧 **Environment Support**: Load `.env` files and environment variable configuration

## Installation

### Pre-built Binaries

Download the latest release from the [GitHub releases page](https://github.com/umbra-ecosystem/de/releases).

### From Source

```bash
cargo install --git https://github.com/umbra-ecosystem/de
```

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
[workspace]
name = "my-workspace"

[project]
name = "my-api"
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

Execute defined tasks from anywhere in your project:

```bash
de run test
de run dev
de run build --release  # Pass additional arguments
```

### 4. Start/Stop Docker Compose Projects

Start all Docker Compose projects in a workspace:

```bash
de start
de start --workspace my-workspace
```

Stop all Docker Compose projects in a workspace:

```bash
de stop
de stop --workspace my-workspace
```

### 5. List Projects

View all projects in your current workspace:

```bash
de list
```

Or list projects in a specific workspace:

```bash
de list --workspace my-workspace
```

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

[tasks]
# Simple shell command
task-name = "command to run"

# Docker Compose service task
service-task = { service = "service-name", command = "command in service" }

# Complex shell command
complex-task = { command = "multi part command with args" }
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

#### Docker Compose Management

Start and stop Docker Compose projects across workspaces:

```bash
# Start all Docker Compose projects in the current workspace
de start

# Start all Docker Compose projects in a specific workspace
de start --workspace production

# Stop all Docker Compose projects in the active workspace
de stop

# Stop all Docker Compose projects in a specific workspace
de stop --workspace production
```

These commands automatically run `docker-compose up -d` and `docker-compose down` respectively for all projects in the workspace that have Docker Compose files configured.

## Examples

### Web Application Project

```toml
[workspace]
name = "web-apps"

[project]
name = "my-blog"
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

### Microservices Workspace

```toml
[workspace]
name = "microservices"

[project]
name = "user-service"

[tasks]
# Local development
dev = "cargo watch -x run"
test = "cargo test"

# Docker tasks
docker-build = "docker build -t user-service ."
docker-test = { service = "user-service", command = "cargo test" }

# Integration tests with full stack
integration = { service = "test-runner", command = "pytest tests/integration" }
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
