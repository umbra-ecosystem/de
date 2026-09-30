//! Matching ticket keys inside branch names. Pure logic, no I/O.

/// Whether `name` contains `key` as a whole token, ignoring case.
///
/// The characters directly around a match must not be letters or digits, so
/// `PROJ-12` does not match `feature/PROJ-123-x` and `PROJ-123` does not match
/// `XPROJ-123-foo`, while `feature/proj-123-add-x`, `PROJ-123/short` and
/// `hotfix/PROJ-123_x` all match.
pub fn name_contains_key(name: &str, key: &str) -> bool {
    if key.is_empty() {
        return false;
    }

    // ASCII lowercasing keeps byte offsets identical to the original strings.
    let haystack = name.to_ascii_lowercase();
    let needle = key.to_ascii_lowercase();
    let bytes = haystack.as_bytes();

    let mut from = 0;
    while let Some(found) = haystack[from..].find(&needle) {
        let start = from + found;
        let end = start + needle.len();

        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }

        // Step past this match start; a later occurrence may have clean boundaries.
        from = start + 1;
        while from < haystack.len() && !haystack.is_char_boundary(from) {
            from += 1;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_matching_table() {
        let cases = [
            ("PROJ-123", "PROJ-123", true),
            ("feature/PROJ-123-x", "PROJ-123", true),
            ("feature/proj-123-add-x", "PROJ-123", true),
            ("PROJ-123/short", "PROJ-123", true),
            ("hotfix/PROJ-123_x", "PROJ-123", true),
            ("feature/PROJ-123", "proj-123", true),
            ("PROJ-123.fix", "PROJ-123", true),
            // longer numbers must not match a shorter key
            ("feature/PROJ-123-x", "PROJ-12", false),
            ("PROJ-1234", "PROJ-123", false),
            ("PROJ-123", "PROJ-12", false),
            // prefix letters
            ("XPROJ-123-foo", "PROJ-123", false),
            ("feature/XPROJ-123", "PROJ-123", false),
            // suffix letters
            ("PROJ-123abc", "PROJ-123", false),
            // a later occurrence with clean boundaries still matches
            ("PROJ-1234-then-PROJ-123", "PROJ-123", true),
            ("XPROJ-123/PROJ-123", "PROJ-123", true),
            ("develop", "PROJ-123", false),
            ("anything", "", false),
            ("feat/ünï-PROJ-123", "PROJ-123", true),
        ];

        for (name, key, expected) in cases {
            assert_eq!(name_contains_key(name, key), expected, "{name} vs {key}");
        }
    }
}
