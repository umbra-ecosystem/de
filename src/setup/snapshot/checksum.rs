use std::{
    fmt::Display,
    fs::File,
    io::{BufReader, Read},
    path::Path,
    str::FromStr,
};

use eyre::{Context, eyre};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use walkdir::WalkDir;

use crate::setup::snapshot::{SNAPSHOT_MANIFEST_FILE, Snapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotChecksum {
    pub algorithm: ChecksumAlgorithm,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Sha256,
}

impl Display for SnapshotChecksum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.algorithm, self.checksum)
    }
}

impl FromStr for ChecksumAlgorithm {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sha256" => Ok(ChecksumAlgorithm::Sha256),
            _ => Err("Unsupported checksum algorithm: only 'sha256' is supported"),
        }
    }
}

impl Display for ChecksumAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChecksumAlgorithm::Sha256 => write!(f, "sha256"),
        }
    }
}

impl<'de> Deserialize<'de> for SnapshotChecksum {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let string = String::deserialize(deserializer)?;
        if let Some((algorithm, checksum)) = string.split_once(':') {
            Ok(SnapshotChecksum {
                algorithm: algorithm.parse().map_err(serde::de::Error::custom)?,
                checksum: checksum.to_string(),
            })
        } else {
            Err(serde::de::Error::custom(
                "Invalid checksum format, expected 'algorithm:checksum'",
            ))
        }
    }
}

impl Serialize for SnapshotChecksum {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = format!("{}:{}", self.algorithm, self.checksum);
        serializer.serialize_str(&s)
    }
}

pub fn calculate_snapshot_checksum(
    algorithm: &ChecksumAlgorithm,
    snapshot: &Snapshot,
    snapshot_dir: &Path,
) -> eyre::Result<SnapshotChecksum> {
    let mut snapshot_clone = snapshot.clone();
    snapshot_clone.checksum = None; // Clear checksum field for hashing

    match algorithm {
        ChecksumAlgorithm::Sha256 => calculate_snapshot_checksum_sha256(&snapshot, snapshot_dir),
    }
}

fn calculate_snapshot_checksum_sha256(
    snapshot: &Snapshot,
    snapshot_dir: &Path,
) -> eyre::Result<SnapshotChecksum> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();

    let serialized = serde_json::to_vec(&snapshot)?;
    hasher.update(&serialized);

    for entry in WalkDir::new(snapshot_dir).max_depth(10) {
        let entry = entry.map_err(|e| eyre!(e)).wrap_err_with(|| {
            format!(
                "Failed to read directory entry in: {}",
                snapshot_dir.display()
            )
        })?;

        let path = entry.path();
        let name = path
            .strip_prefix(snapshot_dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to strip prefix '{}' from path '{}'",
                    snapshot_dir.display(),
                    path.display()
                )
            })?
            .to_str()
            .ok_or_else(|| eyre!("Non UTF-8 Path: {path:?}"))?;

        // Skip the manifest file itself
        if name == SNAPSHOT_MANIFEST_FILE {
            continue;
        }

        if path.is_file() {
            hash_file(&mut hasher, path).wrap_err_with(|| {
                format!(
                    "Failed to hash file '{}' for snapshot checksum",
                    path.display()
                )
            })?;
        }
    }

    let checksum = format!("{:x}", hasher.finalize());

    Ok(SnapshotChecksum {
        algorithm: ChecksumAlgorithm::Sha256,
        checksum,
    })
}

pub enum SnapshotVerification {
    Valid,
    Invalid,
    NoChecksum,
}

pub fn verify_snapshot_checksum(
    snapshot: &Snapshot,
    snapshot_dir: &Path,
) -> eyre::Result<SnapshotVerification> {
    let Some(snapshot_checksum) = &snapshot.checksum else {
        return Ok(SnapshotVerification::NoChecksum);
    };

    let calculated_checksum =
        calculate_snapshot_checksum(&snapshot_checksum.algorithm, snapshot, snapshot_dir)?;

    if calculated_checksum == *snapshot_checksum {
        Ok(SnapshotVerification::Valid)
    } else {
        Ok(SnapshotVerification::Invalid)
    }
}

fn hash_file<D>(hasher: &mut D, path: &Path) -> eyre::Result<()>
where
    D: Digest,
{
    let file = File::open(path)
        .map_err(|e| eyre::eyre!(e))
        .wrap_err_with(|| format!("Failed to open file for hashing: {}", path.display()))?;

    let mut reader = BufReader::new(file);
    let mut buffer = [0; 8192];

    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| eyre::eyre!(e))
            .wrap_err_with(|| format!("Failed to read file for hashing: {}", path.display()))?;

        if n == 0 {
            break;
        }

        hasher.update(&buffer[..n]);
    }

    Ok(())
}
