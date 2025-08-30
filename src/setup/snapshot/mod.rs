mod apply;
mod create;
mod types;

pub use apply::apply_snapshot;
pub use create::create_snapshot;
pub use types::Snapshot;

pub const SNAPSHOT_MANIFEST_FILE: &str = "manifest.json";
