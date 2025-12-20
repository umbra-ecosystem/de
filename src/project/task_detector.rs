use eyre::{Result, eyre};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Source of a detected task
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskSource {
    PackageJson,
    Makefile,
    Justfile,
    Cargo,
    PyProject,
}

impl TaskSource {
    pub fn display_name(&self) -> &str {
        match self {
            TaskSource::PackageJson => "package.json",
            TaskSource::Makefile => "Makefile",
            TaskSource::Justfile => "justfile",
            TaskSource::Cargo => "Cargo.toml",
            TaskSource::PyProject => "pyproject.toml",
        }
    }

    #[allow(dead_code)]
    pub fn priority(&self) -> u8 {
        match self {
            TaskSource::PackageJson => 1,
            TaskSource::Makefile => 2,
            TaskSource::Justfile => 3,
            TaskSource::Cargo => 4,
            TaskSource::PyProject => 5,
        }
    }
}

/// A task detected from project configuration files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedTask {
    pub name: String,
    pub command: String,
    pub source: TaskSource,
    pub description: Option<String>,
}

impl DetectedTask {
    pub fn new(name: String, command: String, source: TaskSource) -> Self {
        Self {
            name,
            command,
            source,
            description: None,
        }
    }

    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }
}

/// Trait for task detectors
pub trait TaskDetector {
    /// Check if this detector can run in the given directory
    fn can_detect(&self, dir: &Path) -> bool;

    /// Detect tasks in the given directory
    fn detect(&self, dir: &Path) -> Result<Vec<DetectedTask>>;

    /// Get the source type for this detector
    fn source(&self) -> TaskSource;
}

/// Detector for package.json npm/yarn/pnpm scripts
pub struct PackageJsonDetector;

impl TaskDetector for PackageJsonDetector {
    fn can_detect(&self, dir: &Path) -> bool {
        dir.join("package.json").exists()
    }

    fn detect(&self, dir: &Path) -> Result<Vec<DetectedTask>> {
        let package_json_path = dir.join("package.json");
        let content = fs::read_to_string(&package_json_path)
            .map_err(|e| eyre!("Failed to read package.json: {}", e))?;

        let json: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| eyre!("Failed to parse package.json: {}", e))?;

        let mut tasks = Vec::new();

        if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
            for (name, _command) in scripts {
                tasks.push(DetectedTask::new(
                    name.clone(),
                    format!("npm run {}", name),
                    TaskSource::PackageJson,
                ));
            }
        }

        Ok(tasks)
    }

    fn source(&self) -> TaskSource {
        TaskSource::PackageJson
    }
}

/// Detector for Makefile targets
pub struct MakefileDetector;

impl TaskDetector for MakefileDetector {
    fn can_detect(&self, dir: &Path) -> bool {
        dir.join("Makefile").exists() || dir.join("makefile").exists()
    }

    fn detect(&self, dir: &Path) -> Result<Vec<DetectedTask>> {
        let makefile_path = if dir.join("Makefile").exists() {
            dir.join("Makefile")
        } else {
            dir.join("makefile")
        };

        let content = fs::read_to_string(&makefile_path)
            .map_err(|e| eyre!("Failed to read Makefile: {}", e))?;

        let mut tasks = Vec::new();

        for line in content.lines() {
            // Skip comments and empty lines
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }

            // Look for target definitions (line that starts at column 0 and contains a colon)
            if !line.starts_with(char::is_whitespace) && line.contains(':') {
                if let Some(target) = line.split(':').next() {
                    let target = target.trim();
                    // Skip special targets and variables
                    if !target.is_empty()
                        && !target.contains('=')
                        && !target.contains('$')
                        && !target.starts_with('.')
                    {
                        tasks.push(DetectedTask::new(
                            target.to_string(),
                            format!("make {}", target),
                            TaskSource::Makefile,
                        ));
                    }
                }
            }
        }

        Ok(tasks)
    }

    fn source(&self) -> TaskSource {
        TaskSource::Makefile
    }
}

/// Detector for justfile recipes
pub struct JustfileDetector;

impl TaskDetector for JustfileDetector {
    fn can_detect(&self, dir: &Path) -> bool {
        dir.join("justfile").exists() || dir.join("Justfile").exists()
    }

    fn detect(&self, dir: &Path) -> Result<Vec<DetectedTask>> {
        let justfile_path = if dir.join("justfile").exists() {
            dir.join("justfile")
        } else {
            dir.join("Justfile")
        };

        let content = fs::read_to_string(&justfile_path)
            .map_err(|e| eyre!("Failed to read justfile: {}", e))?;

        let mut tasks = Vec::new();
        let mut current_comment: Option<String> = None;

        for line in content.lines() {
            let trimmed = line.trim();

            // Capture comment for next recipe
            if trimmed.starts_with('#') {
                current_comment = Some(trimmed[1..].trim().to_string());
                continue;
            }

            // Skip empty lines
            if trimmed.is_empty() {
                current_comment = None;
                continue;
            }

            // Look for recipe definitions (doesn't start with whitespace, contains ':')
            if !line.starts_with(char::is_whitespace) && line.contains(':') {
                if let Some(recipe) = line.split(':').next() {
                    let recipe = recipe.trim();
                    // Skip recipes that start with underscore (private by convention)
                    if !recipe.is_empty() && !recipe.starts_with('_') && !recipe.contains('=') {
                        let mut task = DetectedTask::new(
                            recipe.to_string(),
                            format!("just {}", recipe),
                            TaskSource::Justfile,
                        );
                        if let Some(desc) = current_comment.take() {
                            task = task.with_description(desc);
                        }
                        tasks.push(task);
                    }
                }
                current_comment = None;
            } else {
                current_comment = None;
            }
        }

        Ok(tasks)
    }

    fn source(&self) -> TaskSource {
        TaskSource::Justfile
    }
}

/// Detector for Cargo.toml common commands
pub struct CargoDetector;

impl TaskDetector for CargoDetector {
    fn can_detect(&self, dir: &Path) -> bool {
        dir.join("Cargo.toml").exists()
    }

    fn detect(&self, _dir: &Path) -> Result<Vec<DetectedTask>> {
        // For Cargo projects, we provide common cargo commands
        let common_commands = vec![
            ("build", "cargo build", "Build the project"),
            ("check", "cargo check", "Check the project for errors"),
            ("test", "cargo test", "Run tests"),
            ("run", "cargo run", "Run the project"),
            ("clean", "cargo clean", "Clean build artifacts"),
            ("doc", "cargo doc", "Build documentation"),
            ("clippy", "cargo clippy", "Run clippy lints"),
            ("fmt", "cargo fmt", "Format code"),
        ];

        let tasks = common_commands
            .into_iter()
            .map(|(name, cmd, desc)| {
                DetectedTask::new(name.to_string(), cmd.to_string(), TaskSource::Cargo)
                    .with_description(desc.to_string())
            })
            .collect();

        Ok(tasks)
    }

    fn source(&self) -> TaskSource {
        TaskSource::Cargo
    }
}

/// Detector for pyproject.toml poetry/pip scripts
pub struct PyProjectDetector;

impl TaskDetector for PyProjectDetector {
    fn can_detect(&self, dir: &Path) -> bool {
        dir.join("pyproject.toml").exists()
    }

    fn detect(&self, dir: &Path) -> Result<Vec<DetectedTask>> {
        let pyproject_path = dir.join("pyproject.toml");
        let content = fs::read_to_string(&pyproject_path)
            .map_err(|e| eyre!("Failed to read pyproject.toml: {}", e))?;

        let mut tasks = Vec::new();

        // Try to parse as TOML
        match toml::from_str::<toml::Value>(&content) {
            Ok(toml) => {
                tracing::debug!("Parsed pyproject.toml successfully");

                // Check for poetry scripts
                if let Some(scripts) = toml
                    .get("tool")
                    .and_then(|t| t.get("poetry"))
                    .and_then(|p| p.get("scripts"))
                    .and_then(|s| s.as_table())
                {
                    tracing::debug!("Found {} poetry scripts", scripts.len());
                    for (name, _) in scripts {
                        tasks.push(DetectedTask::new(
                            name.clone(),
                            format!("poetry run {}", name),
                            TaskSource::PyProject,
                        ));
                    }
                }

                // Check for poe tasks (poethepoet)
                if let Some(poe_tasks) = toml
                    .get("tool")
                    .and_then(|t| t.get("poe"))
                    .and_then(|p| p.get("tasks"))
                    .and_then(|t| t.as_table())
                {
                    tracing::debug!("Found {} poe tasks", poe_tasks.len());
                    for (name, _) in poe_tasks {
                        tasks.push(DetectedTask::new(
                            name.clone(),
                            format!("poe {}", name),
                            TaskSource::PyProject,
                        ));
                    }
                }

                // If no specific scripts found but pyproject.toml exists, provide common Python commands
                if tasks.is_empty() {
                    tracing::debug!("No poetry/poe tasks found, providing defaults");
                    tasks = vec![
                        DetectedTask::new(
                            "test".to_string(),
                            "pytest".to_string(),
                            TaskSource::PyProject,
                        )
                        .with_description("Run tests with pytest".to_string()),
                        DetectedTask::new(
                            "lint".to_string(),
                            "ruff check .".to_string(),
                            TaskSource::PyProject,
                        )
                        .with_description("Run linter".to_string()),
                        DetectedTask::new(
                            "format".to_string(),
                            "black .".to_string(),
                            TaskSource::PyProject,
                        )
                        .with_description("Format code".to_string()),
                    ];
                }
            }
            Err(e) => {
                tracing::warn!("Failed to parse pyproject.toml as TOML: {}", e);
            }
        }

        Ok(tasks)
    }

    fn source(&self) -> TaskSource {
        TaskSource::PyProject
    }
}

/// Main task detector that coordinates all individual detectors
pub struct TaskDetectorRegistry {
    detectors: Vec<Box<dyn TaskDetector>>,
}

impl Default for TaskDetectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskDetectorRegistry {
    pub fn new() -> Self {
        let detectors: Vec<Box<dyn TaskDetector>> = vec![
            Box::new(PackageJsonDetector),
            Box::new(MakefileDetector),
            Box::new(JustfileDetector),
            Box::new(CargoDetector),
            Box::new(PyProjectDetector),
        ];

        Self { detectors }
    }

    /// Detect all tasks in the given directory
    pub fn detect_all(&self, dir: &Path) -> Result<BTreeMap<String, DetectedTask>> {
        let mut all_tasks = BTreeMap::new();

        for detector in &self.detectors {
            if detector.can_detect(dir) {
                match detector.detect(dir) {
                    Ok(tasks) => {
                        for task in tasks {
                            // Only add if not already present (first detector wins)
                            all_tasks.entry(task.name.clone()).or_insert(task);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to detect tasks from {}: {}",
                            detector.source().display_name(),
                            e
                        );
                    }
                }
            }
        }

        Ok(all_tasks)
    }

    /// Detect tasks from a specific source
    #[allow(dead_code)]
    pub fn detect_from_source(&self, dir: &Path, source: TaskSource) -> Result<Vec<DetectedTask>> {
        for detector in &self.detectors {
            if detector.source() == source && detector.can_detect(dir) {
                return detector.detect(dir);
            }
        }

        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_package_json_detector() {
        let temp_dir = TempDir::new().unwrap();
        let package_json = r#"
        {
            "name": "test-project",
            "scripts": {
                "test": "jest",
                "build": "webpack",
                "start": "node index.js"
            }
        }
        "#;
        fs::write(temp_dir.path().join("package.json"), package_json).unwrap();

        let detector = PackageJsonDetector;
        assert!(detector.can_detect(temp_dir.path()));

        let tasks = detector.detect(temp_dir.path()).unwrap();
        assert_eq!(tasks.len(), 3);
        assert!(tasks.iter().any(|t| t.name == "test"));
        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "start"));
    }

    #[test]
    fn test_makefile_detector() {
        let temp_dir = TempDir::new().unwrap();
        let makefile = r#"
.PHONY: test build

test:
	go test ./...

build:
	go build -o bin/app

clean:
	rm -rf bin/
"#;
        fs::write(temp_dir.path().join("Makefile"), makefile).unwrap();

        let detector = MakefileDetector;
        assert!(detector.can_detect(temp_dir.path()));

        let tasks = detector.detect(temp_dir.path()).unwrap();
        assert!(tasks.iter().any(|t| t.name == "test"));
        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "clean"));
    }

    #[test]
    fn test_cargo_detector() {
        let temp_dir = TempDir::new().unwrap();
        let cargo_toml = r#"
[package]
name = "test-project"
version = "0.1.0"
"#;
        fs::write(temp_dir.path().join("Cargo.toml"), cargo_toml).unwrap();

        let detector = CargoDetector;
        assert!(detector.can_detect(temp_dir.path()));

        let tasks = detector.detect(temp_dir.path()).unwrap();
        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));
        assert!(tasks.iter().any(|t| t.name == "run"));
    }
}
