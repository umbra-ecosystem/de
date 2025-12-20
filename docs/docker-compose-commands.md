# Docker Compose Commands - Quick Reference

This guide covers all Docker Compose commands available in `de`, which work seamlessly in both workspace mode and no-config mode.

## Overview

All Docker Compose commands in `de` support two modes of operation:

1. **Workspace Mode**: Operate on all projects in a workspace
2. **No-Config Mode**: Operate on the current/inferred project (no `de.toml` required)

## Commands

### `de start`

Start Docker Compose services.

**Usage:**
```bash
# Start current project and its dependencies
de start

# Start all projects in active workspace
de start --workspace

# Start specific workspace
de start --workspace my-workspace
```

**Options:**
- `-w, --workspace [NAME]` - Workspace to start (optional name)
- `-y, --yes` - Skip confirmation prompts

**Works in no-config mode**: ✅ Yes

---

### `de stop`

Stop Docker Compose services.

**Usage:**
```bash
# Stop current project
de stop

# Stop all projects in workspace
de stop --workspace my-workspace
```

**Options:**
- `-w, --workspace <NAME>` - Workspace to stop
- `-y, --yes` - Skip confirmation prompts

**Works in no-config mode**: ✅ Yes

---

### `de logs`

View logs from Docker Compose services.

**Usage:**
```bash
# View logs from current project
de logs

# Follow logs in real-time
de logs --follow
de logs -f

# Show last 100 lines
de logs --tail 100

# Filter by specific service
de logs --service api

# Combine options
de logs -f --service web --tail 50

# Workspace mode
de logs --workspace my-workspace
```

**Options:**
- `-w, --workspace <NAME>` - Operate on workspace
- `-s, --service <NAME>` - Filter by specific service
- `-f, --follow` - Follow log output
- `-t, --tail <N>` - Number of lines to show from end
- `[args]...` - Additional docker compose logs arguments

**Examples:**
```bash
# Follow logs for a specific service
de logs -f --service api

# Show last 50 lines with timestamps
de logs --tail 50 -- --timestamps

# View logs for all services in workspace
de logs --workspace my-workspace --follow
```

**Works in no-config mode**: ✅ Yes

---

### `de restart`

Restart Docker Compose services.

**Usage:**
```bash
# Restart all services
de restart

# Restart specific service
de restart --service api

# Restart services in workspace
de restart --workspace my-workspace
```

**Options:**
- `-w, --workspace <NAME>` - Operate on workspace
- `-s, --service <NAME>` - Restart specific service only
- `[args]...` - Additional docker compose restart arguments

**Examples:**
```bash
# Restart a specific service
de restart --service web

# Restart all services in workspace
de restart --workspace my-workspace

# Restart with timeout
de restart -- --timeout 30
```

**Works in no-config mode**: ✅ Yes

---

### `de ps`

Show status of Docker Compose containers.

**Usage:**
```bash
# Show containers for current project
de ps

# Show all containers including stopped
de ps -- --all

# Show containers for workspace
de ps --workspace my-workspace
```

**Options:**
- `-w, --workspace <NAME>` - Operate on workspace
- `[args]...` - Additional docker compose ps arguments

**Examples:**
```bash
# Show all containers
de ps -- --all

# Show with services info
de ps -- --services

# Format output
de ps -- --format json
```

**Works in no-config mode**: ✅ Yes

---

### `de down`

Stop and remove containers, networks, and optionally volumes.

**Usage:**
```bash
# Stop and remove containers
de down

# Also remove volumes
de down --volumes
de down -v

# Workspace mode with volume removal
de down --workspace my-workspace --volumes
```

**Options:**
- `-w, --workspace <NAME>` - Operate on workspace
- `-v, --volumes` - Remove named volumes
- `[args]...` - Additional docker compose down arguments

**Examples:**
```bash
# Remove everything including volumes
de down -v

# Remove orphan containers
de down -- --remove-orphans

# Complete teardown in workspace
de down --workspace my-workspace --volumes
```

**Works in no-config mode**: ✅ Yes

**⚠️ Warning**: Using `--volumes` will delete all data in named volumes. Use with caution in production-like environments.

---

### `de pull`

Pull the latest images for Docker Compose services.

**Usage:**
```bash
# Pull all images for current project
de pull

# Pull specific service image
de pull --service api

# Pull images for workspace
de pull --workspace my-workspace
```

**Options:**
- `-w, --workspace <NAME>` - Operate on workspace
- `-s, --service <NAME>` - Pull specific service image
- `[args]...` - Additional docker compose pull arguments

**Examples:**
```bash
# Pull a specific service
de pull --service database

# Pull with quiet output
de pull -- --quiet

# Pull all images in workspace
de pull --workspace my-workspace
```

**Works in no-config mode**: ✅ Yes

---

### `de build`

Build or rebuild service images.

**Usage:**
```bash
# Build all services
de build

# Build without cache
de build --no-cache

# Build specific service
de build --service api

# Build workspace projects
de build --workspace my-workspace --no-cache
```

**Options:**
- `-w, --workspace <NAME>` - Operate on workspace
- `-s, --service <NAME>` - Build specific service
- `--no-cache` - Build without using cache
- `[args]...` - Additional docker compose build arguments

**Examples:**
```bash
# Force complete rebuild
de build --no-cache

# Build with parallel builds
de build -- --parallel

# Build specific service without cache
de build --service api --no-cache

# Build all workspace projects
de build --workspace my-workspace
```

**Works in no-config mode**: ✅ Yes

---

## No-Config Mode

All Docker Compose commands work without requiring a `de.toml` configuration file. They automatically detect Docker Compose files in this order of precedence:

1. `compose.yaml`
2. `compose.yml`
3. `docker-compose.yaml`
4. `docker-compose.yml`

### Smart Project Root Detection

`de` automatically finds your project root by searching upward for:
1. `.git` directory (highest priority)
2. Docker Compose files (fallback)

This means you can run commands from any subdirectory of your project.

### Example No-Config Workflow

```bash
# Clone any project with docker-compose.yaml
git clone https://github.com/user/project.git
cd project

# Start services - no configuration needed!
de start

# View logs
de logs -f

# Check status
de ps

# Restart a service
de restart --service api

# Build images
de build --no-cache

# Clean up
de down -v
```

## Workspace Mode

When working with multiple projects, use workspace mode to operate on all projects at once:

```bash
# Start all projects in workspace
de start --workspace my-workspace

# View logs from all projects
de logs --workspace my-workspace --follow

# Restart all services across all projects
de restart --workspace my-workspace

# Build all projects
de build --workspace my-workspace --no-cache

# Complete teardown
de down --workspace my-workspace --volumes
```

## Tips & Best Practices

### 1. Use Follow Mode for Active Development

```bash
de logs -f --service api
```

This keeps logs streaming in real-time, perfect for watching application output during development.

### 2. Tail for Quick Checks

```bash
de logs --tail 50
```

Quickly see recent log entries without overwhelming output.

### 3. Service-Specific Operations

```bash
# Restart just one service
de restart --service worker

# Build just one service
de build --service api --no-cache

# View logs for one service
de logs -f --service web
```

### 4. Clean Rebuilds

When things go wrong, force a complete rebuild:

```bash
de down -v
de build --no-cache
de start
```

### 5. Pass Through Docker Compose Flags

All commands support passing additional flags to docker compose:

```bash
# Pass timestamps flag to logs
de logs -- --timestamps

# Pass quiet flag to pull
de pull -- --quiet

# Pass parallel flag to build
de build -- --parallel
```

### 6. Workspace-Wide Updates

Keep all projects in sync:

```bash
# Pull latest images for all projects
de pull --workspace my-workspace

# Rebuild all projects
de build --workspace my-workspace --no-cache

# Restart everything
de restart --workspace my-workspace
```

## Common Workflows

### Development Workflow

```bash
# Start everything
de start

# Watch logs during development
de logs -f

# Restart after code changes (if needed)
de restart --service api

# Rebuild after dependency changes
de build --service api --no-cache
de restart --service api
```

### Debugging Workflow

```bash
# Check container status
de ps

# View recent logs
de logs --tail 100

# View service-specific logs
de logs --service api --tail 50

# Restart problematic service
de restart --service api
```

### Update Workflow

```bash
# Pull latest images
de pull

# Rebuild if needed
de build --no-cache

# Restart with new images
de restart
```

### Cleanup Workflow

```bash
# Stop everything
de stop

# Or complete teardown
de down

# Nuclear option - remove everything including volumes
de down -v
```

## Error Messages

### No Compose File Found

```
Error: No docker-compose file found. Searched for:
  - compose.yaml
  - compose.yml
  - docker-compose.yaml
  - docker-compose.yml
```

**Solution**: Ensure you have a Docker Compose file in the project root or run from the correct directory.

### No Project Found

```
Error: No project found in current directory
```

**Solution**: Run the command from a project directory with a Docker Compose file, or create a `de.toml` configuration.

## Comparison with Docker Compose CLI

| de Command | Docker Compose Equivalent |
|------------|--------------------------|
| `de start` | `docker compose up -d` |
| `de stop` | `docker compose stop` |
| `de logs -f` | `docker compose logs -f` |
| `de restart` | `docker compose restart` |
| `de ps` | `docker compose ps` |
| `de down` | `docker compose down` |
| `de down -v` | `docker compose down --volumes` |
| `de pull` | `docker compose pull` |
| `de build` | `docker compose build` |

**Benefits of using `de`:**
- Works across multiple projects (workspace mode)
- Simpler, more intuitive commands
- Automatic project/compose file detection
- Consistent interface with other `de` commands
- No need to remember `-f` flags for compose file locations

## Related Commands

- `de status` - Show project and git status
- `de doctor` - Check environment health
- `de exec` - Run commands in project context
- `de task list` - View available tasks

## Further Reading

- [Phase 4 Implementation Summary](./phase-4-implementation-summary.md)
- [No-Config Mode Guide](./no-config-mode.md) (if exists)
- [README](../README.md)

---

**Note**: All commands work in both workspace mode and no-config mode. Choose the mode that fits your workflow!