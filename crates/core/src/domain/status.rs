//! Small closed enums stored as text in SQLite.
//!
//! Each has a stable lowercase text form (`as_str` / `FromStr`) that the database CHECK
//! constraints in `store/migrations.rs` mirror; a store test keeps the two in sync.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Returned when text is not one of an enum's known values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what}: {value:?}")]
pub struct ParseEnumError {
    pub what: &'static str,
    pub value: String,
}

macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident, $what:literal, {
            $( $(#[$vmeta:meta])* $variant:ident => $text:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The text stored in the database.
            pub fn as_str(self) -> &'static str {
                match self {
                    $( $name::$variant => $text ),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseEnumError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $text => Ok($name::$variant), )+
                    other => Err(ParseEnumError { what: $what, value: other.into() }),
                }
            }
        }
    };
}

string_enum! {
    /// Where a ticket is in the author's own workflow. Local only; never sent to Jira.
    ///
    /// Intended lifecycle (see GOAL.md); [`LocalStatus::can_transition_to`] is the exact table:
    ///
    /// - `Claimed`: picked from the Review column, nothing done yet.
    ///   Next: `Reviewing`, `Active` (testing without a recorded review), `Done`.
    /// - `Reviewing`: the PR diff is being read (any number of tickets at once).
    ///   Next: `Active`, `Parked`, `Claimed` (undo), `Done`.
    /// - `Active`: checked out for local test. **At most one ticket at a time**; a unique
    ///   partial index in the database enforces it.
    ///   Next: `Parked` (deactivate), `Integrated` (merged and pushed to `uat`), `Done`.
    /// - `Parked`: testing started, set aside with notes and checklist intact.
    ///   Next: `Active`, `Reviewing`, `Done`.
    /// - `Integrated`: on `uat`, waiting on alpha/UAT.
    ///   Next: `Active` (fix after alpha needs a re-test), `Reviewing` (Returned to Review), `Done`.
    /// - `Done`: finished or dropped. Next: `Claimed` (it came back to Review; treat as new).
    ///
    /// `Integrated` is only reachable from `Active`: you integrate what you tested.
    LocalStatus, "ticket status", {
        Claimed => "claimed",
        Reviewing => "reviewing",
        Active => "active",
        Parked => "parked",
        Integrated => "integrated",
        Done => "done",
    }
}

impl LocalStatus {
    /// Whether moving from `self` to `to` is a legal step. Staying put is not a transition.
    ///
    /// This only checks the state machine; the single-`Active` rule depends on other rows
    /// and is checked by the store (and enforced by the database).
    pub fn can_transition_to(self, to: LocalStatus) -> bool {
        use LocalStatus::*;
        matches!(
            (self, to),
            (Claimed, Reviewing | Active | Done)
                | (Reviewing, Active | Parked | Claimed | Done)
                | (Active, Parked | Integrated | Done)
                | (Parked, Active | Reviewing | Done)
                | (Integrated, Active | Reviewing | Done)
                | (Done, Claimed)
        )
    }
}

string_enum! {
    /// Normal tickets target `develop`; hotfixes target the production branch.
    ///
    /// The real kind is derived from synced PR destinations later; this is only the
    /// optional manual override stored locally.
    TicketKind, "ticket kind", {
        Normal => "normal",
        Hotfix => "hotfix",
    }
}

string_enum! {
    /// How a repo came to be linked to a ticket.
    RepoLinkOrigin, "repo link origin", {
        /// Discovered by matching the ticket key in a branch or PR. Refreshed by each discovery.
        Auto => "auto",
        /// Added by hand. Never overwritten or removed by discovery.
        Manual => "manual",
        /// Hidden by hand (stale or duplicate branch). Discovery must not bring it back.
        Excluded => "excluded",
    }
}

string_enum! {
    /// Result of an audited action.
    AuditOutcome, "audit outcome", {
        Success => "success",
        Failure => "failure",
        Skipped => "skipped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LocalStatus::*;

    #[test]
    fn transition_table_is_exhaustive() {
        let allowed = [
            (Claimed, Reviewing),
            (Claimed, Active),
            (Claimed, Done),
            (Reviewing, Active),
            (Reviewing, Parked),
            (Reviewing, Claimed),
            (Reviewing, Done),
            (Active, Parked),
            (Active, Integrated),
            (Active, Done),
            (Parked, Active),
            (Parked, Reviewing),
            (Parked, Done),
            (Integrated, Active),
            (Integrated, Reviewing),
            (Integrated, Done),
            (Done, Claimed),
        ];

        for &from in LocalStatus::ALL {
            for &to in LocalStatus::ALL {
                assert_eq!(
                    from.can_transition_to(to),
                    allowed.contains(&(from, to)),
                    "{from} -> {to}"
                );
            }
        }
    }

    #[test]
    fn no_status_transitions_to_itself() {
        for &s in LocalStatus::ALL {
            assert!(!s.can_transition_to(s), "{s}");
        }
    }

    #[test]
    fn integrating_requires_being_active() {
        for &s in LocalStatus::ALL {
            assert_eq!(s.can_transition_to(Integrated), s == Active, "{s}");
        }
    }

    #[test]
    fn text_round_trips() {
        for &s in LocalStatus::ALL {
            assert_eq!(s.as_str().parse::<LocalStatus>().unwrap(), s);
        }
        for &k in TicketKind::ALL {
            assert_eq!(k.as_str().parse::<TicketKind>().unwrap(), k);
        }
        for &o in RepoLinkOrigin::ALL {
            assert_eq!(o.to_string().parse::<RepoLinkOrigin>().unwrap(), o);
        }
        for &o in AuditOutcome::ALL {
            assert_eq!(o.as_str().parse::<AuditOutcome>().unwrap(), o);
        }
    }

    #[test]
    fn unknown_text_is_rejected() {
        let err = "Active".parse::<LocalStatus>().unwrap_err();
        assert_eq!(err.what, "ticket status");
        assert!("".parse::<TicketKind>().is_err());
    }
}
