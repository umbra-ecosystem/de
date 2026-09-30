//! The error type every provider method returns.

use std::fmt;

/// Why a provider call failed, in terms sync and the UI can act on.
///
/// The split that matters: [`ProviderError::is_environmental`] errors (not installed, not
/// logged in, offline) mean "nothing is wrong with `de`; show a status and keep the cache",
/// while the rest (a parse failure, a failing command, a missing item) are per-call
/// problems or bugs worth reporting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// The tool (or the adapter for it) is not available.
    #[error("{tool} is not installed: {detail}")]
    NotInstalled { tool: String, detail: String },

    /// The tool is installed but not logged in (or the login expired).
    #[error("{tool} is not authenticated: {detail}")]
    NotAuthenticated { tool: String, detail: String },

    /// The tool or the remote system cannot do this (an old version, a missing feature).
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// The tool ran but its output was not what the adapter expects.
    #[error("could not parse {what} from {tool}: {detail}")]
    Parse {
        tool: String,
        what: String,
        detail: String,
    },

    /// The tool ran and exited unsuccessfully (or could not be started).
    #[error("{tool} failed{}: {stderr}", status.map(|s| format!(" (exit {s})")).unwrap_or_default())]
    Command {
        tool: String,
        /// Exit code; `None` when killed by a signal or never started.
        status: Option<i32>,
        stderr: String,
    },

    /// The remote system could not be reached (offline, DNS, timeout, 5xx).
    #[error("network error: {0}")]
    Network(String),

    /// The requested item does not exist (deleted ticket, unknown PR).
    #[error("not found: {0}")]
    NotFound(String),

    /// Anything else.
    #[error("{0}")]
    Other(String),
}

/// The short category of a [`ProviderError`], for reports and rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderErrorKind {
    NotInstalled,
    NotAuthenticated,
    Unsupported,
    Parse,
    Command,
    Network,
    NotFound,
    Other,
    /// Not a provider error at all: a local failure (database, configuration).
    Local,
}

impl ProviderErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderErrorKind::NotInstalled => "not installed",
            ProviderErrorKind::NotAuthenticated => "not logged in",
            ProviderErrorKind::Unsupported => "unsupported",
            ProviderErrorKind::Parse => "parse error",
            ProviderErrorKind::Command => "command failed",
            ProviderErrorKind::Network => "offline",
            ProviderErrorKind::NotFound => "not found",
            ProviderErrorKind::Other => "error",
            ProviderErrorKind::Local => "local error",
        }
    }
}

impl fmt::Display for ProviderErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ProviderError {
    pub fn kind(&self) -> ProviderErrorKind {
        match self {
            ProviderError::NotInstalled { .. } => ProviderErrorKind::NotInstalled,
            ProviderError::NotAuthenticated { .. } => ProviderErrorKind::NotAuthenticated,
            ProviderError::Unsupported(_) => ProviderErrorKind::Unsupported,
            ProviderError::Parse { .. } => ProviderErrorKind::Parse,
            ProviderError::Command { .. } => ProviderErrorKind::Command,
            ProviderError::Network(_) => ProviderErrorKind::Network,
            ProviderError::NotFound(_) => ProviderErrorKind::NotFound,
            ProviderError::Other(_) => ProviderErrorKind::Other,
        }
    }

    /// True when retrying now is pointless because of the environment (not installed, not
    /// logged in, offline): sync stops talking to that provider and keeps its cache.
    pub fn is_environmental(&self) -> bool {
        matches!(
            self,
            ProviderError::NotInstalled { .. }
                | ProviderError::NotAuthenticated { .. }
                | ProviderError::Network(_)
        )
    }

    pub fn not_installed(tool: impl Into<String>, detail: impl Into<String>) -> Self {
        ProviderError::NotInstalled {
            tool: tool.into(),
            detail: detail.into(),
        }
    }

    pub fn not_authenticated(tool: impl Into<String>, detail: impl Into<String>) -> Self {
        ProviderError::NotAuthenticated {
            tool: tool.into(),
            detail: detail.into(),
        }
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environmental_errors_are_offline_auth_and_missing_tool() {
        assert!(ProviderError::Network("x".into()).is_environmental());
        assert!(ProviderError::not_installed("acli", "x").is_environmental());
        assert!(ProviderError::not_authenticated("bkt", "x").is_environmental());
        for e in [
            ProviderError::Unsupported("x".into()),
            ProviderError::NotFound("x".into()),
            ProviderError::Other("x".into()),
            ProviderError::Command {
                tool: "bkt".into(),
                status: Some(1),
                stderr: "boom".into(),
            },
            ProviderError::Parse {
                tool: "bkt".into(),
                what: "prs".into(),
                detail: "eof".into(),
            },
        ] {
            assert!(!e.is_environmental(), "{e}");
        }
    }

    #[test]
    fn messages_are_readable() {
        let e = ProviderError::Command {
            tool: "bkt".into(),
            status: Some(2),
            stderr: "boom".into(),
        };
        assert_eq!(e.to_string(), "bkt failed (exit 2): boom");
        assert_eq!(e.kind(), ProviderErrorKind::Command);
        let e = ProviderError::Command {
            tool: "bkt".into(),
            status: None,
            stderr: "spawn".into(),
        };
        assert_eq!(e.to_string(), "bkt failed: spawn");
    }
}
