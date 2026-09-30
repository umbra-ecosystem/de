//! Identifiers. Each is its own type, so a repo name cannot be passed where a ticket key is expected.
//!
//! They deref to `str` for reading only; construction is explicit (`TicketKey::new`, `.into()` from text).

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::ops::Deref;

macro_rules! name_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
        impl From<&$name> for $name {
            fn from(s: &$name) -> Self {
                s.clone()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }
        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }
    };
}

name_type!(
    /// A Jira key such as `PROJ-142`. Orders by project, then numerically (`PROJ-9` before `PROJ-10`).
    TicketKey
);
name_type!(
    /// A repository of the workspace, by its project name.
    RepoName
);
name_type!(
    /// A git branch name.
    Branch
);
name_type!(
    /// A comment draft.
    DraftId
);
name_type!(
    /// A next-action suggestion: `<ticket>:<rule>[:<detail>]`, stable across recomputation.
    SuggestionId
);

impl TicketKey {
    fn parts(&self) -> (&str, u32) {
        match self.0.rsplit_once('-') {
            Some((p, n)) => (p, n.parse().unwrap_or(0)),
            None => (&self.0, 0),
        }
    }
}

impl Ord for TicketKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts()
            .cmp(&other.parts())
            .then_with(|| self.0.cmp(&other.0))
    }
}

impl PartialOrd for TicketKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RepoName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
impl PartialOrd for RepoName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SuggestionId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
impl PartialOrd for SuggestionId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Branch {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
impl PartialOrd for Branch {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for DraftId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
impl PartialOrd for DraftId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A pull request number within its repo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PrNumber(pub u32);

impl fmt::Display for PrNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where an inline comment attaches: a line of the new file, or of the old one for a deleted line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LineAnchor {
    New(u32),
    Old(u32),
}

impl LineAnchor {
    pub fn line(self) -> u32 {
        match self {
            LineAnchor::New(n) | LineAnchor::Old(n) => n,
        }
    }
}

impl fmt::Display for LineAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineAnchor::New(n) => write!(f, "new line {n}"),
            LineAnchor::Old(n) => write!(f, "old line {n}"),
        }
    }
}

/// One hunk of one file of one pull request, for the "viewed" marks.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HunkId {
    pub ticket: TicketKey,
    pub pr: PrNumber,
    /// In the "since your review" view rather than the full pull request.
    pub since: bool,
    pub path: String,
    pub hunk: usize,
}
