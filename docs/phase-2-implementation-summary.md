# Phase 2 Implementation Summary: Task Auto-Detection

**Date**: 2025-12-20  
**Status**: ✅ Complete  
**Branch**: Phase 2 of the no-config mode implementation

---

## Overview

Phase 2 extends the no-config mode functionality by adding intelligent task auto-detection from common project configuration files. Users can now discover and run tasks without defining them in `de.toml`, making `de` more useful out-of-the-box.

---

## Implementation Details

### 1. Core Task Detection Module

**File**: `de/src/project/task_detector.rs` (493 lines)

Created a comprehensive task detection system with the following components:

#### Types
- **`TaskSource`** enum - Identifies the source of detected tasks:
  - `PackageJson` - npm/yarn/pnpm scripts
  - `Makefile` - make targets
  - `Justfile` - just recipes
  - `Cargo` - common cargo commands
  - `PyProject` - poetry/poe scripts
  
- **`DetectedTask`** struct - Represents a discovered task:
  ```rust
  pub struct DetectedTask {
      pub name: String,
      pub command: String,
      pub source: TaskSource,
      pub description: Option<String>,
  }
  ```

- **`TaskDetector`** trait - Interface for implementing task detectors:
  ```rust
  pub trait TaskDetector {
      fn can_detect(&self, dir: &Path) -> bool;
      fn detect(&self, dir: &Path) -> Result<Vec<DetectedTask>>;
      fn source(&self) -> TaskSource;
  }
  ```

#### Detector Implementations

1. **PackageJsonDetector**
   - Parses `package.json` using `serde_json`
   - Extracts all scripts from the `"scripts"` field
   - Wraps commands as `npm run <script-name>`

2. **MakefileDetector**
   - Reads `Makefile` or `makefile`
   - Parses target definitions (lines starting at column 0 with `:`)
   - Filters out special targets (`.PHONY`, variables, etc.)
   - Wraps commands as `make <target>`

3. **JustfileDetector**
   - Reads `justfile` or `Justfile`
   - Parses recipe definitions
   - Captures comments above recipes as descriptions
   - Ignores private recipes (starting with `_`)
   - Wraps commands as `just <recipe>`

4. **CargoDetector**
   - Detects presence of `Cargo.toml`
   - Provides common cargo commands with descriptions:
     - build, check, test, run, clean, doc, clippy, fmt

5. **PyProjectDetector**
   - Parses `pyproject.toml` using `toml::from_str`
   - Detects poetry scripts from `[tool.poetry.scripts]`
   - Detects poe tasks from `[tool.poe.tasks]`
   - Provides fallback common Python commands if no scripts found:
     - test (pytest), lint (ruff), format (black)

**Important Fix**: Changed from `content.parse::<toml::Value>()` to `toml::from_str::<toml::Value>(&content)` to properly parse TOML files with dotted table names (toml 0.9.0 requirement).

#### Registry

**`TaskDetectorRegistry`** - Coordinates all detectors:
- Maintains a collection of all detector implementations
- `detect_all()` - Runs all applicable detectors and merges results
- First detector wins on name conflicts (by source priority)
- Gracefully handles detector errors with tracing warnings

### 2. Project Integration

**File**: `de/src/project/mod.rs`

Added:
- Public export of task detection types
- `Project::detect_tasks()` method that creates a registry and detects tasks in the project directory
- Returns `BTreeMap<String, DetectedTask>` for easy lookup

### 3. Command Updates

#### `de task list` (`de/src/commands/task/list.rs`)

Enhanced to show both configured and detected tasks:
- Uses `Project::current_or_inferred()` for no-config mode support
- Displays configured tasks first with `[Configured tasks in project: <name>]` header
- Groups detected tasks by source with `[Detected tasks in project: <name>]` header
- Shows source file name for each group (e.g., "from package.json")
- Displays commands and descriptions for clarity
- Improved formatting with themed output
- Shows helpful tip when no tasks found, listing all supported sources

**Example Output**:
```
Configured tasks in project: my-api
  test (cargo test)
  dev (docker-compose exec api cargo watch -x run)

Detected tasks in project: my-api
  from Cargo.toml
    build (cargo build) - Build the project
    check (cargo check) - Check the project for errors
    clippy (cargo clippy) - Run clippy lints
  from Makefile
    install (make install)
    clean (make clean)
  from package.json
    test (npm run test)
    build (npm run build)
```

#### `de run` (`de/src/commands/run.rs`)

Enhanced to execute detected tasks:
- Changed to use `Project::current_or_inferred()` for no-config mode support
- After checking configured tasks, falls back to detected tasks
- Prints source when running detected tasks: `"Running detected task 'X' from Y"`
- Parses detected task command and executes with proper working directory
- Supports passing additional arguments to detected tasks
- Maintains backward compatibility - configured tasks always take precedence

### 4. Documentation Updates

#### CHANGELOG.md
- Added comprehensive Phase 2 section under `[Unreleased]`
- Listed all detection sources and capabilities
- Noted precedence rules and no-config mode integration

#### README.md
- Added task auto-detection to feature list
- Created new section "3a. Task Auto-Detection" in Quick Start
- Listed all supported task sources with examples
- Provided sample output showing grouped detected tasks
- Documented precedence rule (configured > detected)
- Updated feature bullets to mention auto-detection

### 5. Tests

Added unit tests in `task_detector.rs`:
- `test_package_json_detector` - Verifies npm script detection
- `test_makefile_detector` - Verifies make target detection
- `test_cargo_detector` - Verifies cargo command detection

Tests use `tempfile` crate to create temporary test directories.

---

## Key Design Decisions

### 1. Precedence Model
**Decision**: Configured tasks always override detected tasks  
**Rationale**: Users expect explicit configuration to win; prevents surprising behavior when adding tasks to `de.toml`

### 2. Graceful Degradation
**Decision**: Detector failures only produce warnings, don't fail entire detection  
**Rationale**: One malformed file shouldn't break task discovery for other sources

### 3. Source Grouping
**Decision**: Group detected tasks by source in display  
**Rationale**: Makes it clear where each task comes from; helps users understand what `de` is detecting

### 4. Command Wrapping
**Decision**: Wrap detected commands with appropriate tool (e.g., `npm run`, `make`, `just`)  
**Rationale**: Ensures commands execute correctly with proper tool invocation

### 5. Default Tasks for Some Sources
**Decision**: Cargo and PyProject (when no scripts) provide common default tasks  
**Rationale**: These ecosystems have standard commands that users expect; improves discoverability

### 6. Description Support
**Decision**: Extract descriptions from comments (justfile) and provide for defaults  
**Rationale**: Rich task descriptions improve UX; justfile comments are de-facto documentation

---

## Testing Results

### Manual Testing Performed

✅ **Package.json detection**
- Detected npm scripts correctly
- Commands wrapped as `npm run <script>`

✅ **Makefile detection**
- Detected make targets correctly
- Filtered out `.PHONY` and special targets
- Commands wrapped as `make <target>`

✅ **Justfile detection**
- Detected recipes correctly
- Captured comment descriptions
- Ignored private recipes (starting with `_`)
- Commands wrapped as `just <recipe>`

✅ **Cargo detection**
- Provided common cargo commands with descriptions
- No parsing needed (static list)

✅ **PyProject detection**
- Detected poetry scripts from `[tool.poetry.scripts]`
- Detected poe tasks from `[tool.poe.tasks]`
- Provided defaults when no scripts found
- Fixed TOML parsing issue (parse vs from_str)

✅ **Multi-source detection**
- Created test project with package.json, Makefile, and justfile
- All sources detected and grouped correctly
- No conflicts or duplicates

✅ **Task execution**
- Ran detected tasks successfully
- Correct "Running detected task..." message displayed
- Errors reported properly when tasks fail

✅ **No-config mode integration**
- Works seamlessly with `Project::current_or_inferred()`
- Task detection works in directories without `de.toml`
- Smart root detection still applies

✅ **Precedence**
- Configured tasks take precedence over detected tasks
- No conflicts when names overlap

### Build Status
- ✅ Compiles without errors
- ⚠️ Only pre-existing warnings remain (unrelated to Phase 2)
- ✅ Unit tests pass

---

## Files Changed

### New Files
- `de/src/project/task_detector.rs` - Complete task detection implementation (493 lines)
- `de/docs/phase-2-implementation-summary.md` - This document

### Modified Files
- `de/src/project/mod.rs` - Added `detect_tasks()` method and exports
- `de/src/commands/task/list.rs` - Enhanced to show detected tasks
- `de/src/commands/run.rs` - Enhanced to execute detected tasks
- `de/CHANGELOG.md` - Added Phase 2 section
- `de/README.md` - Added task auto-detection documentation

### Test Files Created
- `de/test-python/pyproject.toml` - Test fixture for Python detection

---

## Known Limitations & Future Improvements

### Current Limitations
1. **Makefile parsing** - Simple line-based parsing; doesn't handle:
   - Multi-line targets
   - Complex variable substitution
   - Conditional targets
   
2. **Justfile parsing** - Basic recipe detection; doesn't handle:
   - Recipe parameters in display
   - Recipe dependencies
   - Complex recipe attributes

3. **No caching** - Tasks detected on every invocation
   - Consider caching detection results for performance
   - Cache invalidation based on file mtime

4. **Description extraction** - Only justfile comments captured
   - Could parse npm scripts descriptions (package.json description field)
   - Could parse Makefile comments above targets

### Potential Future Enhancements

1. **Additional Sources**
   - `Taskfile.yml` (go-task)
   - `composer.json` (PHP)
   - `build.gradle` / `build.gradle.kts` (Gradle)
   - `CMakeLists.txt` (CMake)
   - `Rakefile` (Ruby)
   - `gulpfile.js` / `Gruntfile.js` (JavaScript task runners)

2. **Smart Defaults**
   - Detect project type and suggest relevant tasks
   - Add context-aware descriptions based on project structure

3. **Task Filtering**
   - Allow users to hide certain detected tasks
   - Pattern-based inclusion/exclusion rules

4. **Integration with Shims**
   - Option to auto-generate shims for detected tasks
   - Security considerations needed (prompt user?)

5. **Task Metadata**
   - Detect task dependencies
   - Estimate execution time
   - Tag tasks by category (test, build, deploy, etc.)

---

## Integration with Phase 1

Phase 2 builds seamlessly on Phase 1 foundations:

- **Smart Root Detection**: Task detection respects the inferred project root (git repo or compose file location)
- **No-Config Mode**: Task detection works whether `de.toml` exists or not
- **Current or Inferred**: All commands use `current_or_inferred()` for unified behavior
- **Graceful Fallback**: When no `de.toml`, show detected tasks; when `de.toml` exists, show both

The two phases work together to provide a complete no-config experience:
1. **Phase 1** - Docker Compose operations work without config
2. **Phase 2** - Task execution works without config

---

## Next Steps (Phase 3)

Potential Phase 3 improvements (from original plan):
1. **Git Command Improvements**
   - Auto-detect git remote when not in `de.toml`
   - Workspace-less git operations in inferred mode
   
2. **Dependency Detection**
   - Infer project dependencies from docker-compose files
   - Auto-detect monorepo structure

3. **Interactive Mode**
   - `de init --from-detected` to generate `de.toml` from detected state
   - Prompt to save detected tasks to config

4. **Performance Optimizations**
   - Cache detection results
   - Parallel detection for multiple sources
   - Lazy detection (only when needed)

---

## Conclusion

Phase 2 successfully implements comprehensive task auto-detection, making `de` immediately useful in any project with common configuration files. The implementation is:

- **Robust** - Handles errors gracefully, doesn't break on malformed files
- **Extensible** - Easy to add new detectors via the trait system
- **User-friendly** - Clear output showing sources and descriptions
- **Backward compatible** - Configured tasks always take precedence
- **Well-integrated** - Works seamlessly with Phase 1 no-config mode

Users can now run `de task list` and `de run <task>` in any project without any configuration, significantly lowering the barrier to entry for `de` adoption.