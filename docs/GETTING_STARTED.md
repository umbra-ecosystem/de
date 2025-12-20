# Getting Started with `de`

Welcome to `de` - a simple yet powerful CLI tool for managing Docker-based development environments!

This guide will help you get up and running quickly, whether you want to use `de` without any configuration or set up a full workspace with multiple projects.

## Table of Contents

- [Installation](#installation)
- [Quick Start (No Configuration Required)](#quick-start-no-configuration-required)
- [Understanding the Modes](#understanding-the-modes)
- [Your First Project](#your-first-project)
- [Creating a Workspace](#creating-a-workspace)
- [Common Workflows](#common-workflows)
- [Next Steps](#next-steps)

## Installation

### Pre-built Binaries

Download the latest release from the [GitHub releases page](https://github.com/umbra-ecosystem/de/releases).

**macOS/Linux:**
```bash
# Download and extract (adjust URL for your platform)
curl -LO https://github.com/umbra-ecosystem/de/releases/latest/download/de-<platform>
chmod +x de-<platform>
sudo mv de-<platform> /usr/local/bin/de
```

**Windows:**
Download the `.exe` file and add it to your PATH.

### From Source

If you have Rust installed:
```bash
cargo install --git https://github.com/umbra-ecosystem/de
```

### Verify Installation

```bash
de --version
```

## Quick Start (No Configuration Required)

The fastest way to start using `de` is with any project that has a Docker Compose file. **No configuration needed!**

### Example: Start Any Docker Project

```bash
# Clone any project with docker-compose.yaml
git clone https://github.com/example/my-app.git
cd my-app

# Start services - that's it!
de start

# View logs
de logs -f

# Check status
de ps

# Restart a service
de restart --service api

# Stop everything
de stop
```

### What Just Happened?

`de` automatically:
- ✅ Detected your Docker Compose file
- ✅ Inferred the project root from your `.git` directory
- ✅ Found your project name from the directory
- ✅ Started your services

**No `de.toml` required!**

### Supported Compose Files

`de` looks for these files (in order):
1. `compose.yaml`
2. `compose.yml`
3. `docker-compose.yaml`
4. `docker-compose.yml`

### Smart Root Detection

`de` finds your project root automatically:
- First checks for `.git` directory (repository boundary)
- Falls back to Docker Compose file location
- Works from any subdirectory!

```bash
# Works from anywhere in your project
cd my-project/src/components/
de status  # Still finds the project root!
```

## Understanding the Modes

`de` operates in different modes depending on your setup:

### 1. No-Config Mode (Zero Setup)

**When**: You have a Docker Compose file but no `de.toml`

**What you get**:
- ✅ Start/stop services
- ✅ View logs
- ✅ Run commands in project context
- ✅ Automatic task detection
- ✅ Git operations
- ✅ Health checks

**Perfect for**: Individual projects, trying `de` for the first time, simple Docker setups

### 2. Configured Project Mode (With `de.toml`)

**When**: You create a `de.toml` in your project

**Additional features**:
- ✅ Custom tasks
- ✅ Project dependencies
- ✅ Workspace integration
- ✅ Project-specific configuration

**Perfect for**: Projects you actively maintain, team environments

### 3. Workspace Mode (Multiple Projects)

**When**: You're working with multiple related projects

**What you get**:
- ✅ All project mode features
- ✅ Manage multiple projects together
- ✅ Cross-project dependencies
- ✅ Workspace-level tasks
- ✅ Unified git operations

**Perfect for**: Microservices, monorepo-style workflows, team coordination

## Your First Project

Let's initialize a project with `de.toml` to unlock more features.

### Step 1: Initialize

```bash
cd my-project
de init
```

You'll be prompted for:
- **Workspace name**: Group this project with others (e.g., "my-workspace")
- **Project name**: Identifier for this project (defaults to directory name)

This creates a `de.toml` file:

```toml
[project]
name = "my-api"
workspace = "my-workspace"
docker_compose = "docker-compose.yml"  # Auto-detected

[tasks]
# Define custom tasks here
```

### Step 2: Define Tasks

Edit your `de.toml` to add tasks:

```toml
[tasks]
# Simple shell command
test = "npm test"

# Docker Compose service command
dev = { service = "api", command = "npm run dev" }

# Complex command
migrate = "npm run db:migrate && npm run db:seed"
```

### Step 3: Run Tasks

```bash
# Run configured tasks
de run test
de run dev
de run migrate

# Or use fallthrough (no 'run' needed)
de test
de dev
```

### Step 4: Use Auto-Detection

Even without defining tasks, `de` detects them:

```bash
# See all available tasks
de task list

# Configured tasks (from de.toml)
# ✓ test
# ✓ dev
# ✓ migrate

# Detected tasks (from package.json)
# → npm:build
# → npm:lint
# → npm:start
```

## Creating a Workspace

When you have multiple related projects, organize them into a workspace.

### Initialize Multiple Projects

```bash
# First project
cd ~/projects/api
de init --workspace my-app --name api

# Second project
cd ~/projects/web
de init --workspace my-app --name web

# Third project
cd ~/projects/worker
de init --workspace my-app --name worker
```

### Workspace Operations

```bash
# Start all projects in workspace
de start --workspace my-app

# View status of all projects
de status --workspace my-app

# Run task across all projects
de exec-all npm install

# Switch all projects to a branch
de git switch feature-branch

# Reset all projects to base branch
de git base-reset
```

### Set Active Workspace

```bash
# Set the active workspace
de workspace config active my-app

# Now you can omit --workspace flag
de start
de status
de stop
```

### Project Dependencies

Define dependencies in `de.toml`:

```toml
[project]
name = "web"
workspace = "my-app"
depends_on = ["api", "database"]

[tasks]
dev = { service = "web", command = "npm run dev" }
```

Now `de start` automatically starts dependencies!

## Common Workflows

### Daily Development

```bash
# Morning: Start everything
cd ~/projects/my-app
de start

# Check what's running
de ps

# View logs
de logs -f --service api

# Make changes, restart a service
de restart --service api

# Run tests
de test

# Evening: Stop everything
de stop
```

### Feature Branch Workflow

```bash
# Switch all projects to feature branch
de git switch feature/new-feature

# Work on feature...

# When done, reset to base branch
de git base-reset
```

### Clean Slate

```bash
# Nuclear option - remove everything
de down --volumes

# Rebuild from scratch
de pull
de build --no-cache
de start
```

### Debugging

```bash
# Check health
de doctor

# View recent logs
de logs --tail 100

# Check specific service
de logs --service api --tail 50

# Run commands in context
de exec npm run diagnose
```

### Onboarding a New Project

```bash
# Clone project
git clone https://github.com/company/new-service.git
cd new-service

# Try it without configuration first
de doctor  # Check if everything looks good
de start   # Just works!

# If you like it, initialize
de init --workspace company --name new-service

# Define some tasks
# Edit de.toml...

# Test it out
de task list
de run test
```

## Next Steps

Now that you're up and running, explore more features:

### Learn More

- **[Docker Compose Commands](./docker-compose-commands.md)** - Complete reference for all compose operations
- **[Task Detection](./phase-2-implementation-summary.md)** - How automatic task detection works
- **[Git Operations](./phase-3-implementation-summary.md)** - Advanced git workflows
- **[README](../README.md)** - Full documentation

### Advanced Topics

- **Workspace Configuration** - `de workspace config --help`
- **Custom Tasks** - Add tasks to `de.toml`
- **Environment Variables** - Use `.env` files
- **Command Shims** - Create aliases with `de shim`
- **Setup Snapshots** - Share workspace configs with `de setup`

### Get Help

```bash
# Command-specific help
de start --help
de task --help
de git --help

# General help
de --help

# Check system health
de doctor
```

### Tips & Tricks

#### Tip 1: Use Fallthrough

Instead of `de run test`, just type:
```bash
de test
```

#### Tip 2: Tab Completion

Set up shell completion for faster workflows:
```bash
# Generate completion (depends on your shell)
de completions bash > /etc/bash_completion.d/de
```

#### Tip 3: Global vs Local Tasks

Define common tasks at workspace level, project-specific tasks in `de.toml`.

#### Tip 4: Skip Git in Some Projects

For projects that don't need git operations:
```toml
[project.git]
enabled = false
```

#### Tip 5: Check Before You Start

```bash
# Always a good habit
de doctor
```

#### Tip 6: Use Follow Mode During Development

```bash
# Keep this running in a terminal
de logs -f
```

## Troubleshooting

### "No docker-compose file found"

**Solution**: Make sure you have one of these files:
- `compose.yaml`
- `compose.yml`
- `docker-compose.yaml`
- `docker-compose.yml`

### "No project found in current directory"

**Solution**: 
- Run from a directory with a compose file, or
- Run `de init` to create a `de.toml`

### "Docker Compose is not available"

**Solution**: Install Docker and Docker Compose:
- https://docs.docker.com/get-docker/
- https://docs.docker.com/compose/install/

### Tasks not showing up

**Solution**:
```bash
# Check what de can see
de task list

# Make sure you're in the right directory
pwd

# Check for typos in de.toml
cat de.toml
```

### Services won't start

**Solution**:
```bash
# Check Docker is running
docker ps

# Check logs for errors
de logs --tail 100

# Try rebuilding
de build --no-cache
```

## What's Next?

You're all set! Here's what most users do next:

1. **Try it on your current project** - No setup needed, just `de start`
2. **Create a `de.toml`** - Unlock custom tasks with `de init`
3. **Add more projects** - Build a workspace for your microservices
4. **Explore automation** - Use `de exec-all` and `de git` commands
5. **Share with your team** - Everyone can use the same setup

**Remember**: Start simple (no-config mode), add configuration as needed!

## Questions?

- 📖 [Full Documentation](../README.md)
- 🐛 [Report Issues](https://github.com/umbra-ecosystem/de/issues)
- 💬 [Discussions](https://github.com/umbra-ecosystem/de/discussions)

Welcome to `de` - enjoy your streamlined development workflow! 🚀