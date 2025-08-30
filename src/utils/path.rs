use std::path::Path;

/// Checks if the given path contains any reverse path traversal components (`..`).
pub fn has_reverse_path_traversal(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return true;
        }
    }
    false
}
