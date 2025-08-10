use chrono::{DateTime, Utc};

pub struct Snapshot {
    pub projects: Vec<SnapshotProject>,
    pub created_at: DateTime<Utc>,
}

pub struct SnapshotProject {
    pub steps: Vec<SnapshotProjectStep>,
}

pub enum SnapshotProjectStep {}
