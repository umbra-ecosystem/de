use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::types::Slug;

/// Represents a dependency graph for projects
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// Map from project name to its dependencies
    dependencies: BTreeMap<Slug, BTreeSet<Slug>>,
    /// All projects in the graph (including dependencies)
    projects: BTreeSet<Slug>,
    /// Explicitly added projects (not just dependencies)
    explicit_projects: BTreeSet<Slug>,
}

#[derive(Debug, thiserror::Error)]
pub enum DependencyGraphError {
    #[error("Circular dependency detected among projects: {0:?}")]
    CircularDependency(Vec<Slug>),
    #[error("Missing dependencies: {0:?}")]
    MissingDependencies(Vec<(Slug, Slug)>),
}

impl DependencyGraph {
    /// Create a new dependency graph
    pub fn new() -> Self {
        Self {
            dependencies: BTreeMap::new(),
            projects: BTreeSet::new(),
            explicit_projects: BTreeSet::new(),
        }
    }

    /// Add a project with its dependencies
    pub fn add_project(&mut self, project: Slug, depends_on: Vec<Slug>) {
        self.projects.insert(project.clone());
        self.explicit_projects.insert(project.clone());

        // Add all dependencies to the projects set
        for dep in &depends_on {
            self.projects.insert(dep.clone());
        }

        // Store dependencies
        self.dependencies
            .insert(project, depends_on.into_iter().collect());
    }

    /// Get all projects in the graph
    #[allow(dead_code)]
    pub fn projects(&self) -> &BTreeSet<Slug> {
        &self.projects
    }

    /// Get dependencies for a specific project
    #[allow(dead_code)]
    pub fn get_dependencies(&self, project: &Slug) -> Option<&BTreeSet<Slug>> {
        self.dependencies.get(project)
    }

    /// Resolve dependencies and return projects in startup order (dependencies first)
    pub fn resolve_startup_order(&self) -> Result<Vec<Slug>, DependencyGraphError> {
        self.topological_sort()
    }

    /// Resolve dependencies and return projects in shutdown order (dependents first)
    pub fn resolve_shutdown_order(&self) -> Result<Vec<Slug>, DependencyGraphError> {
        let mut startup_order = self.topological_sort()?;
        startup_order.reverse();
        Ok(startup_order)
    }

    /// Perform topological sort using Kahn's algorithm
    fn topological_sort(&self) -> Result<Vec<Slug>, DependencyGraphError> {
        // Calculate in-degree for each project (number of dependencies)
        let mut in_degree = BTreeMap::new();
        for project in &self.projects {
            in_degree.insert(project.clone(), 0);
        }

        // Count incoming edges - each project's in-degree equals number of dependencies
        for (project, deps) in &self.dependencies {
            if let Some(degree) = in_degree.get_mut(project) {
                *degree = deps.len();
            }
        }

        // Find all projects with no dependencies (in-degree 0)
        let mut queue = VecDeque::new();
        for (project, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(project.clone());
            }
        }

        let mut result = Vec::new();
        let mut processed = BTreeSet::new();

        while let Some(project) = queue.pop_front() {
            result.push(project.clone());
            processed.insert(project.clone());

            // For each project that depends on the current project, decrease its in-degree
            for (dependent, deps) in &self.dependencies {
                if deps.contains(&project)
                    && let Some(degree) = in_degree.get_mut(dependent)
                {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }

        // Check for cycles
        if result.len() != self.projects.len() {
            let remaining: Vec<_> = self.projects.difference(&processed).collect();
            return Err(DependencyGraphError::CircularDependency(
                remaining.into_iter().cloned().collect::<Vec<_>>(),
            ));
        }

        Ok(result)
    }

    /// Check if there are any missing dependencies
    pub fn validate_dependencies(&self) -> Result<(), DependencyGraphError> {
        let mut missing_deps = Vec::new();

        for (project, deps) in &self.dependencies {
            for dep in deps {
                // Check if dependency is not in the explicitly added projects
                if !self.explicit_projects.contains(dep) {
                    missing_deps.push((project.clone(), dep.clone()));
                }
            }
        }

        if !missing_deps.is_empty() {
            return Err(DependencyGraphError::MissingDependencies(missing_deps));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn slug(s: &str) -> Slug {
        Slug::from_str(s).unwrap()
    }

    #[test]
    fn test_simple_dependency_chain() {
        let mut graph = DependencyGraph::new();
        graph.add_project(slug("a"), vec![]);
        graph.add_project(slug("b"), vec![slug("a")]);
        graph.add_project(slug("c"), vec![slug("b")]);

        let startup_order = graph.resolve_startup_order().unwrap();
        assert_eq!(startup_order, vec![slug("a"), slug("b"), slug("c")]);

        let shutdown_order = graph.resolve_shutdown_order().unwrap();
        assert_eq!(shutdown_order, vec![slug("c"), slug("b"), slug("a")]);
    }

    #[test]
    fn test_multiple_dependencies() {
        let mut graph = DependencyGraph::new();
        graph.add_project(slug("a"), vec![]);
        graph.add_project(slug("b"), vec![]);
        graph.add_project(slug("c"), vec![slug("a"), slug("b")]);

        let startup_order = graph.resolve_startup_order().unwrap();
        // a and b can be in any order, but c must come last
        assert!(
            startup_order.iter().position(|x| x == &slug("c")).unwrap()
                > startup_order.iter().position(|x| x == &slug("a")).unwrap()
        );
        assert!(
            startup_order.iter().position(|x| x == &slug("c")).unwrap()
                > startup_order.iter().position(|x| x == &slug("b")).unwrap()
        );
    }

    #[test]
    fn test_circular_dependency() {
        let mut graph = DependencyGraph::new();
        graph.add_project(slug("a"), vec![slug("b")]);
        graph.add_project(slug("b"), vec![slug("a")]);

        let result = graph.resolve_startup_order();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Circular dependency")
        );
    }

    #[test]
    fn test_missing_dependency() {
        let mut graph = DependencyGraph::new();
        graph.add_project(slug("a"), vec![slug("missing")]);

        let result = graph.validate_dependencies();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Missing dependencies")
        );
    }

    #[test]
    fn test_complex_graph() {
        let mut graph = DependencyGraph::new();
        graph.add_project(slug("db"), vec![]);
        graph.add_project(slug("cache"), vec![]);
        graph.add_project(slug("api"), vec![slug("db"), slug("cache")]);
        graph.add_project(slug("web"), vec![slug("api")]);
        graph.add_project(slug("worker"), vec![slug("db"), slug("cache")]);

        let startup_order = graph.resolve_startup_order().unwrap();

        // db and cache should come before api and worker
        let db_pos = startup_order.iter().position(|x| x == &slug("db")).unwrap();
        let cache_pos = startup_order
            .iter()
            .position(|x| x == &slug("cache"))
            .unwrap();
        let api_pos = startup_order
            .iter()
            .position(|x| x == &slug("api"))
            .unwrap();
        let worker_pos = startup_order
            .iter()
            .position(|x| x == &slug("worker"))
            .unwrap();
        let web_pos = startup_order
            .iter()
            .position(|x| x == &slug("web"))
            .unwrap();

        assert!(db_pos < api_pos);
        assert!(cache_pos < api_pos);
        assert!(db_pos < worker_pos);
        assert!(cache_pos < worker_pos);
        assert!(api_pos < web_pos);
    }
}
