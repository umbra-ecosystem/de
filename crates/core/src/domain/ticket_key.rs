//! Jira ticket keys such as `PROJ-123`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid ticket key {0:?}: expected PROJECT-123")]
pub struct ParseTicketKeyError(String);

/// A validated, upper-cased Jira key: `PROJ-123`.
///
/// The project part is ASCII letters and digits starting with a letter; the number has no
/// leading zero (Jira never issues one).
///
/// Ordered by project, then by number numerically (`PROJ-9 < PROJ-10`), not as text. Equality
/// and hashing stay on the normalised text, which agrees with this order: two keys compare
/// equal exactly when project and number are the same.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TicketKey(String);

impl Ord for TicketKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // The number has no leading zero, so a longer digit string is the larger number
        // (and no overflow is possible, unlike parsing to an integer).
        let number = |k: &TicketKey| k.0.split_once('-').map_or("", |(_, n)| n).to_owned();
        let (a, b) = (number(self), number(other));
        self.project()
            .cmp(other.project())
            .then_with(|| a.len().cmp(&b.len()).then_with(|| a.cmp(&b)))
    }
}

impl PartialOrd for TicketKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn valid_project(p: &str) -> bool {
    p.starts_with(|c: char| c.is_ascii_alphabetic()) && p.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn valid_number(n: &str) -> bool {
    !n.starts_with('0') && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
}

impl TicketKey {
    /// The normalised (upper-case) key.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The project part, e.g. `PROJ`.
    pub fn project(&self) -> &str {
        self.0.split_once('-').map_or("", |(p, _)| p)
    }

    /// The numeric part, e.g. `123`.
    pub fn number(&self) -> u64 {
        self.0
            .split_once('-')
            .and_then(|(_, n)| n.parse().ok())
            .unwrap_or(0)
    }

    /// Finds every ticket key inside arbitrary text (a branch name, a PR title), ignoring case.
    ///
    /// Keys are matched as whole tokens: the project part is a maximal run of letters and
    /// digits and the number a maximal run of digits, so `PROJ-12` is not found inside
    /// `PROJ-123` and `XPROJ-123` yields `XPROJ-123`, not `PROJ-123`. Any non-alphanumeric
    /// character (`/`, `_`, space, `.`) separates tokens. Results are de-duplicated and keep
    /// the order of first appearance.
    ///
    /// Words that look like keys (`release-2024`) also match; callers decide by checking
    /// the project against Jira.
    pub fn find_in(text: &str) -> Vec<TicketKey> {
        // Maximal runs of ASCII letters/digits, as byte ranges.
        let bytes = text.as_bytes();
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_alphanumeric() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                runs.push((start, i));
            } else {
                i += 1;
            }
        }

        let mut found: Vec<TicketKey> = Vec::new();
        let mut r = 0;
        while r + 1 < runs.len() {
            let (ps, pe) = runs[r];
            let (ns, ne) = runs[r + 1];
            // A key is `project` + a single '-' + `number`, with nothing else attached.
            let adjacent = ns == pe + 1 && bytes[pe] == b'-';
            if adjacent && valid_project(&text[ps..pe]) && valid_number(&text[ns..ne]) {
                let key = TicketKey(format!(
                    "{}-{}",
                    text[ps..pe].to_ascii_uppercase(),
                    &text[ns..ne]
                ));
                if !found.contains(&key) {
                    found.push(key);
                }
                r += 2;
            } else {
                r += 1;
            }
        }
        found
    }
}

impl FromStr for TicketKey {
    type Err = ParseTicketKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once('-') {
            Some((project, number)) if valid_project(project) && valid_number(number) => Ok(
                TicketKey(format!("{}-{number}", project.to_ascii_uppercase())),
            ),
            _ => Err(ParseTicketKeyError(s.into())),
        }
    }
}

impl TryFrom<String> for TicketKey {
    type Error = ParseTicketKeyError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<TicketKey> for String {
    fn from(key: TicketKey) -> String {
        key.0
    }
}

impl fmt::Display for TicketKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> TicketKey {
        s.parse().unwrap()
    }

    fn find(text: &str) -> Vec<String> {
        TicketKey::find_in(text)
            .into_iter()
            .map(|k| k.to_string())
            .collect()
    }

    #[test]
    fn parses_and_normalises() {
        assert_eq!(key("PROJ-123").as_str(), "PROJ-123");
        assert_eq!(key("proj-123").as_str(), "PROJ-123");
        assert_eq!(key("Ab1-9").to_string(), "AB1-9");
        assert_eq!(key("proj-42").project(), "PROJ");
        assert_eq!(key("proj-42").number(), 42);
    }

    #[test]
    fn keys_sort_by_project_then_number_not_as_text() {
        use std::cmp::Ordering;
        assert!(key("PROJ-9") < key("PROJ-10"));
        assert!(key("PROJ-2") < key("PROJ-100"));
        assert!(key("PROJ-99999999999999999999") > key("PROJ-9"));
        assert!(key("ABC-500") < key("PROJ-1"));
        assert!(key("AB-1") < key("ABC-1"));
        assert_eq!(key("proj-7").cmp(&key("PROJ-7")), Ordering::Equal);
        assert_eq!(
            key("PROJ-7").partial_cmp(&key("PROJ-8")),
            Some(Ordering::Less)
        );

        let mut keys: Vec<TicketKey> =
            ["PROJ-10", "ZED-1", "PROJ-9", "PROJ-100", "ABC-3", "PROJ-9"]
                .iter()
                .map(|k| key(k))
                .collect();
        keys.sort();
        let sorted: Vec<&str> = keys.iter().map(TicketKey::as_str).collect();
        assert_eq!(
            sorted,
            ["ABC-3", "PROJ-9", "PROJ-9", "PROJ-10", "PROJ-100", "ZED-1"]
        );
        // BTree collections follow the same order.
        let set: std::collections::BTreeSet<TicketKey> = keys.into_iter().collect();
        let in_set: Vec<&str> = set.iter().map(TicketKey::as_str).collect();
        assert_eq!(in_set, ["ABC-3", "PROJ-9", "PROJ-10", "PROJ-100", "ZED-1"]);
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "PROJ",
            "123",
            "PROJ-",
            "-123",
            "PROJ-abc",
            "PROJ 123",
            " PROJ-123",
            "PROJ-123 ",
            "PROJ-12-3",
            "PROJ--1",
            "PR OJ-1",
            "1PROJ-1",
            "PROJ-0",
            "PROJ-012",
            "PR_OJ-1",
            "PROJ-1a",
            "ÉPROJ-1",
        ] {
            assert!(
                bad.parse::<TicketKey>().is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn serde_uses_the_string_form() {
        let json = serde_json::to_string(&key("proj-1")).unwrap();
        assert_eq!(json, "\"PROJ-1\"");
        assert_eq!(
            serde_json::from_str::<TicketKey>("\"proj-2\"").unwrap(),
            key("PROJ-2")
        );
        assert!(serde_json::from_str::<TicketKey>("\"nope\"").is_err());
    }

    #[test]
    fn finds_keys_in_branch_names() {
        assert_eq!(find("feature/proj-123-add-x"), ["PROJ-123"]);
        assert_eq!(find("PROJ-123"), ["PROJ-123"]);
        assert_eq!(find("bugfix/PROJ-123"), ["PROJ-123"]);
        assert_eq!(find("hotfix_proj-9_login"), ["PROJ-9"]);
        assert_eq!(find("PROJ-123: fix (PROJ-124)"), ["PROJ-123", "PROJ-124"]);
    }

    #[test]
    fn boundaries_are_respected() {
        // The shorter number must not match inside a longer one.
        assert_eq!(find("feature/PROJ-123-x"), ["PROJ-123"]);
        // A longer prefix wins over the embedded project.
        assert_eq!(find("XPROJ-123"), ["XPROJ-123"]);
        assert_eq!(find("feature/xproj-7"), ["XPROJ-7"]);
        // Letters glued onto the number mean it is not a key.
        assert!(find("PROJ-123abc").is_empty());
        // Digits-first tokens are not projects, and chained numbers are one key.
        assert_eq!(find("PROJ-123-456"), ["PROJ-123"]);
        assert!(find("2024-01-15").is_empty());
        assert!(find("PROJ-0123").is_empty());
    }

    #[test]
    fn find_dedupes_and_keeps_order() {
        assert_eq!(
            find("b-2 A-1 proj-9 B-2 a-1 PROJ-9 c-3"),
            ["B-2", "A-1", "PROJ-9", "C-3"]
        );
    }

    #[test]
    fn find_handles_no_keys_and_odd_input() {
        assert!(find("").is_empty());
        assert!(find("main").is_empty());
        assert!(find("PROJ-").is_empty());
        assert!(find("-123").is_empty());
        assert!(find("PROJ - 123").is_empty());
        assert_eq!(find("é/proj-5/ü"), ["PROJ-5"]);
    }
}
