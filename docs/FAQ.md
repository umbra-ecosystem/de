# Frequently Asked Questions (FAQ)

## General Questions

### What is `de`?

`de` is a CLI tool for managing Docker-based development environments. It simplifies working with Docker Compose projects, whether you're working on a single project or managing multiple related services.

### Do I need a configuration file to use `de`?

**No!** `de` works without any configuration. Just navigate to a project with a Docker Compose file and run commands like `de start`, `de logs`, etc. 

You can optionally create a `de.toml` file to unlock additional features like custom tasks, project dependencies, and workspace management.

### What's the difference between no-config mode and configured mode?

**No-Config Mode** (no `de.toml`):
- Works with any Docker Compose project
- Auto-detects compose files and project structure
- Supports all Docker Compose operations
- Automatic task detection from build files

**Configured Mode** (with `de.toml`):
- All no-config features, plus:
- Custom task definitions
- Project dependencies
- Workspace integration
- Fine-grained configuration

Start with no-config mode, add configuration as you need more features!

### How is `de` different from Docker Compose?

`de` wraps Docker Compose and adds:
- 🔍 **Auto-detection** - No need to specify `-f docker-compose.yml` every time
- 🏗️ **Multi-project management** - Manage multiple related projects as a workspace
- ⚡ **Task system** - Define and run common tasks easily
- 🔗 **Dependencies** - Automatically start dependent services
- 🔀 **Git operations** - Manage branches across multiple repos
- 🩺 **Health checks** - Diagnose issues with `de doctor`

You can still use `docker compose` directly - `de` is complementary!

### Can I use `de` with existing Docker Compose projects?

**Absolutely!** `de` works with any standard Docker Compose project without modification. Just run `de start` in your project directory.

## Installation & Setup

### How do I install `de`?

**Pre-built binaries**: Download from [GitHub releases](https://github.com/umbra-ecosystem/de/releases)

**From source** (requires Rust):
```bash
cargo install --git https://github.com/umbra-ecosystem/de
```

**Update**:
```bash
de self update
```

### Which Docker Compose files does `de` recognize?

`de` automatically detects these files (in order of precedence):
1. `compose.yaml` (recommended, modern format)
2. `compose.yml`
3. `docker-compose.yaml` (legacy format)
4. `docker-compose.yml`

### How do I specify a different compose file?

If you have multiple compose files or a non-standard name, specify it in `de.toml`:

```toml
[project]
name = "my-project"
docker_compose = "docker-compose.prod.yml"
```

### Do I need to create a workspace?

**No.** Workspaces are optional and only needed when managing multiple related projects together. For single projects, just use `de` directly - no workspace needed!

## Project & Workspace Management

### How do I initialize a project?

```bash
cd my-project
de init
```

You'll be prompted for workspace and project names. This creates a `de.toml` file.

### Can I have multiple workspaces?

**Yes!** Each workspace is independent and can contain different sets of projects.

```bash
# Create projects in different workspaces
de init --workspace personal --name blog
de init --workspace work --name api
de init --workspace work --name frontend

# Switch between workspaces
de workspace config active work
```

### How do I add an existing project to a workspace?

```bash
cd existing-project
de init --workspace my-workspace --name my-project
```

Even if the project already has a compose file, `de init` will create the `de.toml` configuration.

### Can a project be in multiple workspaces?

**No.** Each project belongs to exactly one workspace. However, you can:
- Clone the project to a different location for another workspace
- Use different branches for different workspaces
- Create a `de.toml` template and modify it for different setups

### What's the difference between project tasks and workspace tasks?

**Project Tasks** (in project's `de.toml`):
- Specific to one project
- Run in project's directory
- Can be Docker Compose service commands

**Workspace Tasks** (in workspace config):
- Run across all projects or at workspace level
- Useful for orchestration
- Managed via `de workspace run`

### How do I remove a project from a workspace?

Simply delete the `de.toml` file from the project:
```bash
cd my-project
rm de.toml
```

The project will still work in no-config mode. To also remove it from workspace listings:
```bash
de scan --workspace my-workspace
```

## Tasks

### What's the difference between `de run` and `de exec`?

**`de run <task>`**: 
- Runs predefined tasks from `de.toml` or detected tasks
- Example: `de run test`, `de run build`

**`de exec <command>`**:
- Runs arbitrary commands in project context
- Example: `de exec npm install`, `de exec python manage.py migrate`

### How do I see all available tasks?

```bash
de task list
```

This shows both:
- Configured tasks (from `de.toml`)
- Detected tasks (from `package.json`, `Makefile`, etc.)

### What files does task auto-detection support?

- `package.json` - npm/yarn/pnpm scripts
- `Makefile` - make targets
- `justfile` - just recipes
- `Cargo.toml` - common cargo commands
- `pyproject.toml` - poetry scripts and poe tasks

### Can I run a task without the `de run` prefix?

**Yes!** Use fallthrough:

```bash
de test        # Same as: de run test
de build       # Same as: de run build
my-api migrate # Same as: de run --project my-api migrate
```

### How do I pass arguments to tasks?

Just append them after the task name:

```bash
de run test -- --verbose
de run migrate -- --force
de test --coverage   # With fallthrough
```

### Can tasks run commands in Docker Compose services?

**Yes!** Define service-based tasks:

```toml
[tasks]
# Run in a Docker Compose service
migrate = { service = "api", command = "npm run db:migrate" }

# Run as shell command (on host)
lint = "npm run lint"
```

### Do detected tasks override configured tasks?

**No.** Configured tasks in `de.toml` always take precedence. This lets you customize behavior of detected tasks if needed.

## Docker Compose Operations

### What Docker Compose commands does `de` support?

All essential commands:
- `de start` - Start services (up -d)
- `de stop` - Stop services
- `de restart` - Restart services
- `de logs` - View logs
- `de ps` - Show container status
- `de down` - Stop and remove containers
- `de pull` - Pull images
- `de build` - Build images

Plus many others! See `de --help` for complete list.

### How do I view logs for a specific service?

```bash
de logs --service api
de logs --service api --follow
de logs --service api --tail 50
```

### Can I pass flags through to Docker Compose?

**Yes!** Use `--` to pass additional flags:

```bash
de logs -- --timestamps
de ps -- --all
de build -- --parallel
```

### What does `de down --volumes` do?

It stops containers and removes networks AND volumes (deletes data). Use with caution!

```bash
de down           # Removes containers and networks
de down --volumes # Also removes volumes (data loss!)
```

### Why use `de start` instead of `docker compose up`?

Benefits of `de start`:
- No need to specify `-f docker-compose.yml`
- Handles project dependencies automatically
- Can start entire workspace with one command
- Shows better status information
- Works from any subdirectory

## Git Operations

### Can I use `de` git commands without a workspace?

**Yes!** Git commands work in three modes:
1. **Single repo** (no workspace): Operates on current git repository
2. **Current project** (in workspace): Operates on current project's repo
3. **Workspace** (full workspace): Operates on all projects

### What does `de git switch` do?

Switches branches across one or all projects:

```bash
# Switch current repo
de git switch feature-branch

# Switch all workspace projects
de git switch feature-branch  # (when workspace is active)

# With fallback branch
de git switch feature-1 --fallback main
```

### What does `de git base-reset` do?

Resets project(s) to a clean state on the base branch:
- Fetches latest changes
- Checks for uncommitted changes
- Checks out base branch
- Hard resets to remote
- Cleans untracked files

**Warning**: This is destructive! Uncommitted changes will be lost (with prompt).

### Can I exclude projects from git operations?

**Yes!** In the project's `de.toml`:

```toml
[project.git]
enabled = false
```

### How does `de` detect the default branch?

`de` checks:
1. Workspace's `default_branch` config
2. Git's remote HEAD (`origin/HEAD`)
3. Falls back to `main`

Set explicitly:
```bash
de workspace config default-branch develop
```

## Troubleshooting

### Why does `de start` say "No docker-compose file found"?

**Possible causes**:
1. No compose file in the directory
2. File named incorrectly
3. Running from wrong directory

**Solution**:
```bash
# Check for compose file
ls -la | grep compose

# Make sure you're in project root
pwd

# If file exists but has different name
de init  # Will auto-detect and set docker_compose path
```

### Tasks aren't being detected

**Solution**:
```bash
# See what de found
de task list

# Check files exist
ls -la package.json Makefile justfile Cargo.toml

# Make sure you're in project root
pwd
```

### `de doctor` shows errors

**Common issues**:

**Docker not running**:
```bash
# Start Docker Desktop, then:
docker ps
```

**Compose file invalid**:
```bash
# Validate manually
docker compose -f compose.yaml config --quiet
```

**Missing dependencies**:
```bash
# Install Docker Compose
# See: https://docs.docker.com/compose/install/
```

### Services won't start

**Debugging steps**:
```bash
# Check what's wrong
de logs --tail 100

# Check if containers exist
de ps -- --all

# Try rebuilding
de down
de build --no-cache
de start
```

### Port conflicts

**Error**: "port is already allocated"

**Solution**:
```bash
# Find what's using the port
lsof -i :8080  # Replace 8080 with your port

# Stop conflicting service, then:
de start
```

### Permission errors on Linux

**Issue**: Docker requires sudo

**Solution**: Add your user to docker group:
```bash
sudo usermod -aG docker $USER
newgrp docker  # Or log out and back in
```

## Advanced Topics

### Can I use environment variables?

**Yes!** `de` supports `.env` files:

```bash
# Create .env file
cat > .env << EOF
DATABASE_URL=postgres://localhost/mydb
API_KEY=secret123
EOF

# Use in compose file
# services:
#   api:
#     environment:
#       - DATABASE_URL=${DATABASE_URL}
```

### Can I have project-specific environment files?

**Yes!** Use `.env.local` for local overrides:

```bash
.env          # Committed to git (defaults)
.env.local    # Ignored by git (local secrets)
```

### How do I create command aliases?

Use shims (Unix-like systems):

```bash
# Create shim for common task
de shim add test

# Now run from anywhere
test  # Same as: de run test
```

### Can I share workspace configuration?

**Yes!** Use setup snapshots:

```bash
# Create snapshot
de setup snapshot.toml

# Someone else applies it
de setup snapshot.toml --target-dir ~/projects
```

### How do I run commands across all workspace projects?

```bash
# Execute in all projects
de exec-all npm install
de exec-all git status
de exec-all make clean
```

### Can I define dependencies between projects?

**Yes!** In `de.toml`:

```toml
[project]
name = "frontend"
depends_on = ["api", "database"]
```

Now `de start` in frontend will start api and database first!

### Can I disable no-config mode?

No-config mode is always available - it's a feature, not a setting. However:
- Configured projects (with `de.toml`) take precedence
- You can be explicit: always use `--project` or `--workspace` flags
- Configure workspace to prevent accidental operations

## Performance & Optimization

### Is `de` slower than using Docker Compose directly?

**No.** `de` is a thin wrapper that calls Docker Compose. The overhead is negligible (milliseconds).

### How do I speed up builds?

```bash
# Use BuildKit (Docker's new build engine)
export DOCKER_BUILDKIT=1

# Parallel builds
de build -- --parallel

# Use layer caching effectively in Dockerfile
```

### Can I cache task detection results?

Task detection is fast (< 100ms typically), but if needed, it's only done when you run `de task list` or execute a detected task.

## Integration & Ecosystem

### Can I use `de` in CI/CD?

**Yes!** `de` works great in CI:

```yaml
# .github/workflows/test.yml
- name: Install de
  run: |
    curl -LO https://github.com/umbra-ecosystem/de/releases/latest/download/de-linux
    chmod +x de-linux && sudo mv de-linux /usr/local/bin/de

- name: Run tests
  run: de run test
```

### Does `de` work with podman?

Not officially tested, but since `de` uses Docker Compose, it should work if you have `podman-compose` and `podman` aliased to `docker`.

### Can I use `de` with remote Docker hosts?

**Yes!** `de` respects Docker's `DOCKER_HOST` environment variable:

```bash
export DOCKER_HOST=tcp://remote-host:2375
de start
```

### Is there IDE integration?

Not built-in, but `de` works great with:
- **VS Code**: Use terminal or tasks.json
- **JetBrains**: Use Run Configurations
- **Vim/Neovim**: Use `:!de` commands

Example VS Code task:
```json
{
  "label": "de start",
  "type": "shell",
  "command": "de start"
}
```

## Contributing & Support

### How do I report a bug?

[Open an issue](https://github.com/umbra-ecosystem/de/issues) with:
- `de --version`
- What you tried
- What happened
- What you expected
- Relevant logs or screenshots

### How do I request a feature?

[Start a discussion](https://github.com/umbra-ecosystem/de/discussions) or open a feature request issue!

### Can I contribute code?

**Yes!** See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines.

### Is there a community?

- [GitHub Discussions](https://github.com/umbra-ecosystem/de/discussions)
- [Issue Tracker](https://github.com/umbra-ecosystem/de/issues)

### How is `de` licensed?

`de` is open source under the MIT License. See [LICENSE](../LICENSE) for details.

## Tips & Best Practices

### Should I commit `de.toml` to git?

**Yes!** `de.toml` defines project configuration and should be shared with your team.

### Should I commit workspace config?

**No.** Workspace configs are stored in `~/.config/de/workspaces/` and are per-user. Use `de setup` snapshots to share workspace structure.

### What's the recommended project structure?

```
my-project/
├── .git/
├── compose.yaml          # Docker Compose file
├── de.toml              # de configuration (optional)
├── .env                 # Environment variables
├── .env.local           # Local overrides (gitignored)
├── src/
└── ...
```

### How do I organize a workspace?

```
~/projects/
├── my-workspace-api/     # Project 1
│   ├── de.toml
│   └── compose.yaml
├── my-workspace-web/     # Project 2
│   ├── de.toml
│   └── compose.yaml
└── my-workspace-worker/  # Project 3
    ├── de.toml
    └── compose.yaml
```

### Should I use workspaces or no-config mode?

**Use no-config mode when**:
- Working on a single project
- Trying `de` for the first time
- Project is simple/standalone
- You prefer minimal setup

**Use workspaces when**:
- Managing multiple related projects (microservices)
- Need cross-project operations
- Team collaboration
- Complex dependencies

**Start simple, add complexity only when needed!**

---

**Still have questions?** 

- 📖 [Read the docs](../README.md)
- 🚀 [Getting Started Guide](./GETTING_STARTED.md)
- 💬 [Ask in Discussions](https://github.com/umbra-ecosystem/de/discussions)