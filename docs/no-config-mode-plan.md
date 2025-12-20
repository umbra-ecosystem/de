# Plan: No-Config Mode for DE

## Overview

Enable `de` to provide useful functionality without requiring a `de.toml` configuration file. This makes `de` more approachable for new users and allows it to work out-of-the-box with existing projects.

## Goals

1. **Lower barrier to entry**: Users can start using `de` immediately without configuration
2. **Progressive enhancement**: Easy path to add more configuration as needs grow
3. **Smart defaults**: Sensible behavior inferred from project structure
4. **Backward compatibility**: Existing `de.toml` projects work exactly as before

## Design Philosophy

Instead of creating separate types for inferred projects, we reuse the existing `ProjectManifest` structure with default/inferred values. This keeps the codebase simple and maintainable.

**Key Insight**: When `de.toml` is missing, create a `ProjectManifest` with:
- `name`: derived from directory name
- `workspace`: "default"
- `docker_compose`: None (auto-detected via existing logic)
- `tasks`: None (or auto-detected in Phase 2)
- `git`: default settings

## Implementation Phases

### Phase 1: Docker Compose Operations (Highest Priority)

**Goal**: Make `de start`, `de stop`, and `de status` work without `de.toml` for projects with Docker Compose files.

#### Core Changes

1. **Add Helper Method to `Slug`** (`src/types.rs` or wherever Slug is defined)
   ```rust
   impl Slug {
       pub fn from_dir_name(dir: &Path) -> eyre::Result<Self> {
           let dir_name = dir
               .file_name()
               .ok_or_else(|| eyre!("Failed to extract directory name"))?
               .to_str()
               .ok_or_else(|| eyre!("Directory name is not valid UTF-8"))?;
           
           Slug::sanitize(dir_name)
               .ok_or_else(|| eyre!("Failed to create valid slug from directory name"))
       }
   }
   ```

2. **Add Inferred Project Support to `Project`** (`src/project/mod.rs`)
   ```rust
   impl Project {
       /// Creates a project from a directory, inferring values if no de.toml exists
       pub fn from_dir_or_inferred(dir: &Path) -> eyre::Result<Self> {
           match Self::from_dir(dir) {
               Ok(project) => Ok(project),
               Err(_) => Self::infer_from_dir(dir),
           }
       }
       
       /// Creates a project with inferred values from the directory structure
       fn infer_from_dir(dir: &Path) -> eyre::Result<Self> {
           // Load .env if exists (same as from_dir)
           let dot_env = dir.join(".env");
           if dot_env.exists() {
               dotenvy::from_path_override(&dot_env)
                   .map_err(|e| eyre!(e))
                   .wrap_err_with(|| {
                       format!("Failed to load environment variables from {}", dir.display())
                   })?;
           }
           
           // Create inferred manifest
           let manifest = ProjectManifest {
               project: ProjectMetadata {
                   name: Slug::from_dir_name(dir)?,
                   workspace: Slug::from_str("default")?,
                   docker_compose: None,  // Auto-detected by docker_compose_path()
                   depends_on: None,
               },
               git: Some(ProjectGitSettings::default()),
               tasks: None,
               setup: None,
           };
           
           Ok(Self {
               dir: dir.to_path_buf(),
               manifest,
               manifest_path: dir.join("de.toml"),  // Path for reference (doesn't exist)
           })
       }
       
       /// Gets the current project, with inferred values if no de.toml exists
       pub fn current_or_inferred() -> eyre::Result<Option<Self>> {
           let current_dir = std::env::current_dir()
               .map_err(|e| eyre!(e))
               .wrap_err("Failed to get current working directory")?;
           
           // Try recursive search first
           if let Some(project) = Self::from_dir_recursive(&current_dir)? {
               return Ok(Some(project));
           }
           
           // Fall back to inferred from current directory
           Ok(Some(Self::infer_from_dir(&current_dir)?))
       }
       
       /// Checks if this project was inferred (no de.toml)
       pub fn is_inferred(&self) -> bool {
           !self.manifest_path.exists()
       }
   }
   ```

3. **Update Commands**

   **`de start` (`src/commands/start.rs`)**
   ```rust
   pub fn start(workspace_name: Option<Option<Slug>>, yes: bool) -> eyre::Result<()> {
       let ui = UserInterface::new();
   
       check_for_active_workspace(&ui, yes)?;
   
       if let Some(workspace_name) = workspace_name {
           // Workspace mode - requires proper configuration
           let workspace = get_workspace_for_cli(Some(workspace_name))
               .map_err(|e| eyre!(e))
               .wrap_err("Failed to get workspace for CLI")?;
   
           spin_up_workspace(&workspace)
               .map_err(|e| eyre!(e))
               .wrap_err("Failed to spin up workspace")?;
   
           Config::mutate_persisted(|config| {
               config.set_active_workspace(Some(workspace.config().name.clone()));
           })?;
   
           ui.new_line()?;
           let _ = workspace_status(&ui, &workspace);
       } else {
           // Current project mode - can use inferred project
           let project = Project::current_or_inferred()
               .map_err(|e| eyre!(e))
               .wrap_err("Failed to get current project")?
               .ok_or_else(|| eyre!("No current project found"))?;
   
           // Show hint if inferred
           if project.is_inferred() {
               ui.hint("💡 Tip: Run 'de init' to create a de.toml for more features (workspace management, tasks, dependencies)")?;
               ui.new_line()?;
           }
   
           // For inferred projects, skip workspace/dependency logic
           if project.is_inferred() {
               ui.writeln(&ui.theme.bold(&format!("Starting {}:", project.manifest().project().name)))?;
               
               let started = project.docker_compose_up()
                   .map_err(|e| eyre!(e))
                   .wrap_err("Failed to start docker compose services")?;
               
               if !started {
                   ui.warning_item("No docker-compose file found", None)?;
               }
           } else {
               // Existing logic for configured projects with workspace/dependencies
               let workspace_name = project.manifest().project().workspace.clone();
               let workspace = Workspace::load_from_name(&workspace_name)
                   .map_err(|e| eyre!(e))
                   .wrap_err("Failed to load workspace")?
                   .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;
   
               spin_up_project_and_dependencies(&ui, &workspace, &project.manifest().project().name)
                   .map_err(|e| eyre!(e))
                   .wrap_err("Failed to spin up project and dependencies")?;
   
               Config::mutate_persisted(|config| {
                   config.set_active_workspace(Some(workspace_name));
               })?;
   
               ui.new_line()?;
               let _ = workspace_status(&ui, &workspace);
           }
       }
   
       Ok(())
   }
   ```

   **`de stop` (`src/commands/stop.rs`)**
   ```rust
   pub fn stop(workspace_name: Option<Slug>, yes: bool) -> eyre::Result<()> {
       let ui = UserInterface::new();
       
       if let Some(workspace_name) = workspace_name {
           // Workspace mode - existing logic
           let workspace = Workspace::load_from_name(&workspace_name)
               .map_err(|e| eyre!(e))
               .wrap_err("Failed to load workspace")?
               .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;
           
           stop_workspace(&ui, workspace, yes)?;
       } else {
           // Current project mode - can use inferred project
           let project = Project::current_or_inferred()
               .map_err(|e| eyre!(e))
               .wrap_err("Failed to get current project")?
               .ok_or_else(|| eyre!("No current project found"))?;
           
           if project.is_inferred() {
               // Simple stop for inferred projects
               ui.writeln(&ui.theme.bold(&format!("Stopping {}:", project.manifest().project().name)))?;
               
               let stopped = project.docker_compose_down()
                   .map_err(|e| eyre!(e))
                   .wrap_err("Failed to stop docker compose services")?;
               
               if !stopped {
                   ui.warning_item("No docker-compose file found", None)?;
               }
           } else {
               // Existing logic for configured projects with workspace
               let workspace = Workspace::active()
                   .map_err(|e| eyre!(e))
                   .wrap_err("Failed to get current workspace")?
                   .ok_or_else(|| eyre!("No workspace is currently active"))?;
               
               stop_workspace(&ui, workspace, yes)?;
           }
       }
       
       Ok(())
   }
   ```

   **`de status` (`src/commands/status.rs`)**
   ```rust
   // Update to show inferred project status gracefully
   // If project.is_inferred(), show a simplified status view
   // Possibly skip workspace-wide checks
   ```

#### Testing Requirements

1. **Unit Tests**
   - `Slug::from_dir_name()` with various directory names
   - `Project::infer_from_dir()` creates correct manifest
   - `Project::is_inferred()` returns correct value

2. **Integration Tests**
   - `de start` in directory with compose.yaml (no de.toml)
   - `de start` in directory with docker-compose.yml (no de.toml)
   - `de start` in directory with no compose file (no de.toml) - should show warning
   - `de stop` in inferred project
   - `de status` in inferred project
   - All existing tests still pass

3. **Manual Testing Scenarios**
   ```bash
   # Create test directory
   mkdir /tmp/test-project
   cd /tmp/test-project
   
   # Create simple compose file
   echo 'services:
     web:
       image: nginx' > compose.yaml
   
   # Try without de.toml
   de start  # Should work!
   de status # Should show services
   de stop   # Should work!
   
   # Now add de.toml
   de init
   de start  # Should still work, but with full features
   ```

#### Documentation Updates

1. **README.md** - Add section "Quick Start (No Configuration)"
   ```markdown
   ## Quick Start (No Configuration)
   
   `de` works out-of-the-box with projects that have Docker Compose files:
   
   ```bash
   cd /path/to/project-with-compose
   de start  # Automatically detects and starts compose services
   de stop   # Stops compose services
   ```
   
   For more features like workspaces, tasks, and dependencies, run `de init` to create a configuration file.
   ```

2. **Add hints in command output** when in inferred mode

#### Estimated Time: 2-3 weeks

**Week 1**: Core implementation
- Days 1-2: Add `Slug::from_dir_name()` and `Project::infer_from_dir()`
- Days 3-4: Update `de start` command
- Day 5: Update `de stop` command

**Week 2**: Testing and refinement
- Days 1-2: Update `de status` command
- Days 3-4: Write comprehensive tests
- Day 5: Manual testing and bug fixes

**Week 3**: Documentation and polish
- Days 1-2: Update documentation
- Days 3-4: Add helpful hints/messages
- Day 5: Code review and refinements

---

### Phase 2: Auto-detected Tasks

**Goal**: Detect and run tasks from common task runners without `de.toml`.

#### Scope

Support auto-detection of tasks from:
1. `package.json` scripts (Node.js/npm/yarn/pnpm)
2. `Makefile` targets (Make)
3. `justfile` recipes (Just)
4. `Cargo.toml` (Rust - standard commands like build, test, run)
5. `pyproject.toml` (Python - poetry/hatch commands)

#### Architecture

1. **Create Task Detection System** (`src/project/task_detector.rs`)
   ```rust
   pub trait TaskDetector: Send + Sync {
       /// Detect tasks in the given directory
       fn detect(&self, dir: &Path) -> eyre::Result<Vec<DetectedTask>>;
       
       /// Priority for conflict resolution (higher = higher priority)
       fn priority(&self) -> u8;
       
       /// Name of this detector
       fn name(&self) -> &'static str;
   }
   
   #[derive(Debug, Clone)]
   pub struct DetectedTask {
       pub name: String,
       pub command: String,
       pub description: Option<String>,
       pub source: &'static str,
   }
   
   pub struct TaskDetectorRegistry {
       detectors: Vec<Box<dyn TaskDetector>>,
   }
   
   impl TaskDetectorRegistry {
       pub fn default() -> Self {
           Self {
               detectors: vec![
                   Box::new(PackageJsonDetector),
                   Box::new(MakefileDetector),
                   Box::new(JustfileDetector),
                   Box::new(CargoDetector),
                   Box::new(PythonDetector),
               ],
           }
       }
       
       pub fn detect_all(&self, dir: &Path) -> eyre::Result<Vec<DetectedTask>> {
           let mut all_tasks = Vec::new();
           
           for detector in &self.detectors {
               match detector.detect(dir) {
                   Ok(tasks) => all_tasks.extend(tasks),
                   Err(_) => continue,  // Ignore detection errors
               }
           }
           
           Ok(all_tasks)
       }
   }
   ```

2. **Implement Detectors**

   **`PackageJsonDetector`** (`src/project/detectors/package_json.rs`)
   ```rust
   pub struct PackageJsonDetector;
   
   impl TaskDetector for PackageJsonDetector {
       fn detect(&self, dir: &Path) -> eyre::Result<Vec<DetectedTask>> {
           let package_json = dir.join("package.json");
           if !package_json.exists() {
               return Ok(Vec::new());
           }
           
           let content = std::fs::read_to_string(&package_json)?;
           let json: serde_json::Value = serde_json::from_str(&content)?;
           
           let scripts = json.get("scripts")
               .and_then(|s| s.as_object())
               .ok_or_else(|| eyre!("No scripts found"))?;
           
           let tasks = scripts
               .iter()
               .map(|(name, command)| DetectedTask {
                   name: name.clone(),
                   command: format!("npm run {}", name),
                   description: command.as_str().map(|s| s.to_string()),
                   source: "package.json",
               })
               .collect();
           
           Ok(tasks)
       }
       
       fn priority(&self) -> u8 { 100 }
       fn name(&self) -> &'static str { "package.json" }
   }
   ```

   **`MakefileDetector`** (`src/project/detectors/makefile.rs`)
   ```rust
   pub struct MakefileDetector;
   
   impl TaskDetector for MakefileDetector {
       fn detect(&self, dir: &Path) -> eyre::Result<Vec<DetectedTask>> {
           let makefile = dir.join("Makefile");
           if !makefile.exists() {
               return Ok(Vec::new());
           }
           
           let content = std::fs::read_to_string(&makefile)?;
           let mut tasks = Vec::new();
           
           // Simple regex to find targets (lines ending with :)
           for line in content.lines() {
               if let Some(target) = line.split(':').next() {
                   let target = target.trim();
                   // Skip internal targets starting with _
                   if !target.is_empty() && !target.starts_with('_') && !target.starts_with('.') {
                       tasks.push(DetectedTask {
                           name: target.to_string(),
                           command: format!("make {}", target),
                           description: None,
                           source: "Makefile",
                       });
                   }
               }
           }
           
           Ok(tasks)
       }
       
       fn priority(&self) -> u8 { 90 }
       fn name(&self) -> &'static str { "Makefile" }
   }
   ```

   **Similar implementations for**: `JustfileDetector`, `CargoDetector`, `PythonDetector`

3. **Update `Project` to Include Detected Tasks**
   ```rust
   impl Project {
       pub fn get_task(&self, name: &str) -> Option<Task> {
           // First, check configured tasks (highest priority)
           if let Some(tasks) = &self.manifest.tasks {
               if let Ok(slug) = Slug::from_str(name) {
                   if let Some(task) = tasks.get(&slug) {
                       return Some(task.clone());
                   }
               }
           }
           
           // Fall back to detected tasks
           if let Ok(detected) = self.detect_tasks() {
               for task in detected {
                   if task.name == name {
                       return Some(Task::Raw(RawTask {
                           command: task.command,
                       }));
                   }
               }
           }
           
           None
       }
       
       pub fn detect_tasks(&self) -> eyre::Result<Vec<DetectedTask>> {
           let registry = TaskDetectorRegistry::default();
           registry.detect_all(&self.dir)
       }
       
       pub fn list_all_tasks(&self) -> eyre::Result<BTreeMap<String, TaskInfo>> {
           let mut tasks = BTreeMap::new();
           
           // Add configured tasks
           if let Some(configured) = &self.manifest.tasks {
               for (name, task) in configured {
                   tasks.insert(
                       name.to_string(),
                       TaskInfo {
                           source: "de.toml",
                           description: task.description(),
                       },
                   );
               }
           }
           
           // Add detected tasks (if not already configured)
           for task in self.detect_tasks()? {
               tasks.entry(task.name.clone()).or_insert_with(|| TaskInfo {
                   source: task.source,
                   description: task.description,
               });
           }
           
           Ok(tasks)
       }
   }
   ```

4. **Update Commands**

   **`de run` (`src/commands/run.rs`)**
   ```rust
   pub fn run(command: Slug, args: Vec<String>, project: Option<Slug>) -> eyre::Result<()> {
       let project = if let Some(project_slug) = project {
           // Workspace-specified project (existing logic)
           // ...
       } else {
           // Current project (can be inferred)
           Project::current_or_inferred()?
               .ok_or_else(|| eyre!("No current project found"))?
       };
       
       // Try to get task (checks both configured and detected)
       let task = project.get_task(command.as_str())
           .ok_or_else(|| eyre!("Task '{}' not found", command))?;
       
       // Execute task (existing logic works)
       task.execute(&project, &args)?;
       
       Ok(())
   }
   ```

   **`de list` (`src/commands/list.rs`)**
   ```rust
   // Update to show both configured and detected tasks
   // Indicate source with icon or color
   // Example output:
   //   dev       [package.json]  npm run dev
   //   test      [de.toml]       cargo test --workspace
   //   build     [Makefile]      make build
   ```

#### Testing Requirements

1. **Unit Tests**
   - Each detector with sample files
   - Task priority/conflict resolution
   - Configured tasks override detected tasks

2. **Integration Tests**
   - `de run` with detected tasks
   - `de list` shows both configured and detected
   - Multiple task sources in same project

3. **Manual Testing**
   ```bash
   # Node.js project
   echo '{"scripts":{"dev":"next dev"}}' > package.json
   de list  # Should show 'dev' task
   de run dev  # Should run npm run dev
   
   # Add de.toml to override
   de init
   # Edit de.toml to add: dev = "echo custom"
   de run dev  # Should run custom command, not npm
   ```

#### Documentation Updates

1. Update README with task detection section
2. Document priority order
3. Show examples with different task runners

#### Estimated Time: 2-3 weeks

**Week 1**: Core detection system
- Days 1-2: Task detection architecture
- Days 3-5: Implement PackageJsonDetector and MakefileDetector

**Week 2**: More detectors and integration
- Days 1-2: JustfileDetector, CargoDetector, PythonDetector
- Days 3-4: Update `de run` and `de list`
- Day 5: Task conflict resolution

**Week 3**: Testing and polish
- Days 1-3: Comprehensive testing
- Days 4-5: Documentation and refinements

---

### Phase 3: Enhanced Git Operations

**Goal**: Make git commands work without `de.toml` on any git repository.

#### Changes

1. **Update `de git switch`**
   - Work on any git repo, even without de.toml
   - Auto-detect remote from git config
   - Skip workspace-wide operations if inferred

2. **Update `de git status`**
   - Show git status for current repo
   - Work without workspace context

#### Implementation
```rust
// Commands check project.is_inferred() and adjust behavior
if project.is_inferred() {
    // Skip workspace-wide operations
    // Work on current repo only
} else {
    // Existing workspace logic
}
```

#### Estimated Time: 1 week

**Days 1-2**: Update git commands for inferred mode
**Days 3-4**: Testing
**Day 5**: Documentation

---

### Phase 4: Polish and Final Touches

**Goal**: Improve UX, messages, documentation, and overall experience.

#### Tasks

1. **Better Error Messages**
   - When de.toml is required but missing
   - Suggest `de init` in appropriate contexts
   - Clear indication of inferred vs configured mode

2. **Helpful Hints**
   - Show tips about `de init` in inferred mode
   - Explain available features in configured mode
   - Progressive disclosure of features

3. **Documentation**
   - Complete README overhaul
   - Add "Getting Started" section
   - Update all examples
   - Create migration guide
   - Add FAQ about inferred mode

4. **Examples**
   - Create example projects for different scenarios
   - Add to repository or docs

5. **Announcement**
   - Blog post about the feature
   - Update changelog
   - Social media announcement

#### Estimated Time: 1-2 weeks

**Week 1**: Messages, hints, and core docs
**Week 2**: Examples, guides, and announcement

---

## Complete Timeline

### Total Duration: 6-10 weeks

| Phase | Duration | Dependencies |
|-------|----------|--------------|
| Phase 1: Docker Compose | 2-3 weeks | None |
| Phase 2: Task Detection | 2-3 weeks | Phase 1 |
| Phase 3: Git Operations | 1 week | Phase 1 |
| Phase 4: Polish | 1-2 weeks | Phases 1-3 |

### Milestones

**M1: Basic Inferred Mode (End of Phase 1)**
- ✓ `de start/stop/status` work without de.toml
- ✓ Docker Compose auto-detection works
- ✓ Basic documentation updated

**M2: Task Detection (End of Phase 2)**
- ✓ `de run` works with detected tasks
- ✓ Multiple task runners supported
- ✓ Task override system works

**M3: Full Inferred Support (End of Phase 3)**
- ✓ Git commands work in inferred mode
- ✓ All core features available

**M4: Production Ready (End of Phase 4)**
- ✓ Comprehensive documentation
- ✓ User-friendly messages
- ✓ Ready for announcement

---

## Success Metrics

### Quantitative
- **Adoption**: % of `de` users who use commands before running `de init`
- **Conversion**: % of users who eventually run `de init`
- **Retention**: % of inferred mode users who continue using `de`
- **Bug Rate**: Number of issues related to inferred mode vs configured mode

### Qualitative
- User feedback on ease of getting started
- Community response to feature announcement
- Support ticket volume (should decrease with better onboarding)

---

## Risks & Mitigation

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Breaking existing projects | High | Low | Extensive testing, backward compatibility checks |
| Performance degradation | Medium | Low | Cache detection results, lazy evaluation |
| Confusion about modes | Medium | Medium | Clear messaging, good documentation |
| Task conflicts | Medium | Medium | Clear priority rules, override system |
| Maintenance burden | Medium | Low | Good abstractions, comprehensive tests |
| Security (detected tasks) | High | Low | Warn about untrusted projects, don't auto-create shims |

---

## Open Questions

1. **Should we cache detected tasks?**
   - Pro: Better performance
   - Con: Stale data if files change
   - **Decision**: Start without caching, add if needed

2. **How aggressive should hints be?**
   - Show on every command?
   - Only once per session?
   - **Decision**: Once per command invocation, but subtle

3. **Should shims work for detected tasks?**
   - Pro: Consistent UX
   - Con: Security risk with untrusted code
   - **Decision**: Phase 2 decision - likely no auto-shims for detected tasks

4. **Should we auto-upgrade to de.toml?**
   - Offer `de init --from-detected` to preserve detected state?
   - **Decision**: Yes, add in Phase 4

5. **What if multiple compose files exist?**
   - We already handle this with priority order
   - **Decision**: Existing logic is fine

---

## Future Enhancements (Post-Launch)

1. **Config Generation from Detected State**
   ```bash
   de init --from-detected
   # Creates de.toml with all detected tasks and settings
   ```

2. **More Task Detectors**
   - Gradle (Java)
   - Maven (Java)
   - CMake (C++)
   - Rake (Ruby)
   - Mix (Elixir)

3. **Smart Workspace Creation**
   ```bash
   de workspace create --scan ../parent-dir
   # Scans for projects and creates workspace
   ```

4. **IDE Integration**
   - LSP for auto-complete of detected tasks
   - VS Code task provider
   - JetBrains plugin

5. **Performance Optimization**
   - Cache detection results
   - Parallel detection
   - Lazy evaluation

---

## Backward Compatibility Guarantee

**All existing functionality must continue to work exactly as before.**

- Projects with de.toml: No changes in behavior
- All existing commands: Work as before with de.toml
- APIs: No breaking changes to public interfaces
- Configuration: All existing de.toml files valid

The only changes are:
1. **New methods** added to `Project` (additive)
2. **Commands work in more contexts** (enhancement)
3. **Better error messages** (improvement)

---

## Definition of Done

For each phase to be considered complete:

- [ ] All code implemented and reviewed
- [ ] All tests passing (unit + integration)
- [ ] Manual testing scenarios verified
- [ ] Documentation updated
- [ ] Changelog updated
- [ ] No regressions in existing functionality
- [ ] Performance acceptable (no significant slowdown)
- [ ] Code reviewed by at least one other developer
- [ ] User-facing messages reviewed for clarity

---

## Next Steps

1. **Review this plan** with team/stakeholders
2. **Get feedback** on approach and timeline
3. **Finalize open questions** (especially around UX decisions)
4. **Create tracking issues** for each phase
5. **Start Phase 1 implementation**

---

## Notes

- This plan assumes existing Docker Compose auto-detection (just implemented) works correctly
- Task detection uses simple parsing - may not handle all edge cases
- Focus is on common use cases first, edge cases later
- Progressive enhancement: each phase adds value independently
- Can ship phases incrementally rather than waiting for all phases

---

**Status**: Draft - Ready for Review
**Last Updated**: 2024
**Author**: Development Team