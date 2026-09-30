//! Editing `composer.json` for the test overlay, and recognising the overlay afterwards.
//!
//! Pure functions over bytes and text; no I/O.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::OverlayError;

/// The `repositories` entry that points composer at a local checkout.
pub fn path_repository(url: &str) -> Value {
    json!({
        "type": "path",
        "url": url,
        "options": { "symlink": true },
    })
}

/// Rewrites `composer.json` so each `(package, url)` resolves to the path repository at `url`:
/// a `path` repository with `symlink` is put in front of the existing ones (composer prefers
/// earlier repositories, so it beats a private registry listed there) and the package's
/// constraint becomes `*` in `require` and `require-dev`, wherever it is required.
///
/// Key order of the whole document is preserved. Indentation and the trailing newline are
/// kept as they were; other whitespace and string escapes are normalised. An identical
/// repository entry is not added twice. Errors when the package is required nowhere.
pub fn apply_overlay(
    original: &[u8],
    entries: &[(String, String)],
) -> Result<Vec<u8>, OverlayError> {
    let text = std::str::from_utf8(original)
        .map_err(|e| OverlayError::InvalidComposerJson(format!("not valid UTF-8: {e}")))?;
    let mut document: Value =
        serde_json::from_str(text).map_err(|e| OverlayError::InvalidComposerJson(e.to_string()))?;
    let root = document.as_object_mut().ok_or_else(|| {
        OverlayError::InvalidComposerJson("the top level is not an object".into())
    })?;

    for (package, url) in entries {
        let mut required = false;
        for section in ["require", "require-dev"] {
            if let Some(constraint) = root
                .get_mut(section)
                .and_then(Value::as_object_mut)
                .and_then(|deps| deps.get_mut(package.as_str()))
            {
                *constraint = Value::String("*".into());
                required = true;
            }
        }
        if !required {
            return Err(OverlayError::PackageNotRequired(package.clone()));
        }

        add_repository(root, package, path_repository(url));
    }

    let mut out = Vec::new();
    let indent = detect_indent(text);
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(&document, &mut serializer)
        .map_err(|e| OverlayError::InvalidComposerJson(e.to_string()))?;
    if text.ends_with('\n') {
        out.push(b'\n');
    }
    Ok(out)
}

fn add_repository(root: &mut Map<String, Value>, package: &str, entry: Value) {
    match root.get_mut("repositories") {
        Some(Value::Array(repos)) => {
            if !repos.contains(&entry) {
                repos.insert(0, entry);
            }
        }
        // The object form is keyed by repository name.
        Some(Value::Object(repos)) => {
            if !repos.values().any(|v| v == &entry) {
                let mut rebuilt = Map::new();
                rebuilt.insert(format!("de-overlay-{package}"), entry);
                for (key, value) in std::mem::take(repos) {
                    rebuilt.entry(key).or_insert(value);
                }
                *repos = rebuilt;
            }
        }
        // Absent (or a value composer would reject anyway): start a list.
        _ => {
            root.insert("repositories".into(), Value::Array(vec![entry]));
        }
    }
}

/// The indentation of the first indented line, four spaces (composer's own) when there is none.
fn detect_indent(text: &str) -> String {
    text.lines()
        .find_map(|line| {
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            (!indent.is_empty() && indent.len() < line.len()).then_some(indent)
        })
        .unwrap_or_else(|| "    ".into())
}

/// What kind of overlay residue a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakKind {
    /// A `"type": "path"` repository with `symlink` in `composer.json`.
    PathRepository,
    /// A constraint of exactly `"*"` for a package the config maps as an overlay package.
    WildcardConstraint,
    /// A `dist` of type `path` in `composer.lock`.
    LockPathDist,
}

impl std::fmt::Display for LeakKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LeakKind::PathRepository => "path repository with symlink",
            LeakKind::WildcardConstraint => "overlay package required as \"*\"",
            LeakKind::LockPathDist => "package installed from a path",
        })
    }
}

/// A line that looks like overlay residue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: LeakKind,
    /// 1-based line number in the file, when known.
    pub line: Option<u32>,
    pub text: String,
}

static TYPE_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""type"\s*:\s*"path""#).expect("valid regex"));
static SYMLINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""symlink"\s*:\s*true"#).expect("valid regex"));

/// Scans lines of a `composer.json` (all of a file, or only the added lines of a diff).
///
/// A path repository counts when a `"type": "path"` line and a `"symlink": true` line both
/// appear (on one line or several); a plain path repository without `symlink` is not the
/// overlay. A `"*"` constraint counts only for the given overlay `packages`.
pub fn scan_composer_json(lines: &[(Option<u32>, &str)], packages: &[&str]) -> Vec<Finding> {
    let mut findings = Vec::new();

    if lines.iter().any(|(_, text)| SYMLINK.is_match(text)) {
        for (line, text) in lines.iter().filter(|(_, text)| TYPE_PATH.is_match(text)) {
            findings.push(finding(LeakKind::PathRepository, *line, text));
        }
    }

    let wildcards: Vec<Regex> = packages
        .iter()
        .map(|package| {
            // composer.json may spell the slash as `\/`.
            let name = regex::escape(package).replace('/', r"\\?/");
            // Any spelling of "match anything": spaces and a stability flag (`*@dev`) included,
            // and composer package names are case-insensitive.
            Regex::new(&format!(r#"(?i)"{name}"\s*:\s*"\s*\*\s*(@\w+)?\s*""#))
                .expect("valid regex")
        })
        .collect();
    for (line, text) in lines {
        if wildcards.iter().any(|re| re.is_match(text)) {
            findings.push(finding(LeakKind::WildcardConstraint, *line, text));
        }
    }

    findings.sort_by_key(|f| f.line);
    findings
}

/// Scans lines of a `composer.lock` for a package installed from a `path`.
pub fn scan_composer_lock(lines: &[(Option<u32>, &str)]) -> Vec<Finding> {
    lines
        .iter()
        .filter(|(_, text)| TYPE_PATH.is_match(text))
        .map(|(line, text)| finding(LeakKind::LockPathDist, *line, text))
        .collect()
}

fn finding(kind: LeakKind, line: Option<u32>, text: &str) -> Finding {
    Finding {
        kind,
        line,
        text: text.trim().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
    "name": "acme/web",
    "require": {
        "php": "^8.2",
        "acme/api-client": "^2.1",
        "monolog/monolog": "^3.0"
    },
    "require-dev": {
        "acme/api-client": "^2.0",
        "phpunit/phpunit": "^11"
    },
    "repositories": [
        {
            "type": "composer",
            "url": "https://packages.acme.test"
        }
    ],
    "config": {
        "sort-packages": true
    }
}
"#;

    fn entries(url: &str) -> Vec<(String, String)> {
        vec![("acme/api-client".into(), url.into())]
    }

    fn edited(original: &str, url: &str) -> Value {
        let out = apply_overlay(original.as_bytes(), &entries(url)).unwrap();
        serde_json::from_slice(&out).expect("output is valid JSON")
    }

    #[test]
    fn edits_repositories_and_constraints_and_keeps_everything_else() {
        let out = apply_overlay(SAMPLE.as_bytes(), &entries("../api-client")).unwrap();
        let text = String::from_utf8(out).unwrap();
        let doc: Value = serde_json::from_str(&text).unwrap();

        assert_eq!(doc["require"]["acme/api-client"], "*");
        assert_eq!(doc["require-dev"]["acme/api-client"], "*");
        assert_eq!(doc["require"]["monolog/monolog"], "^3.0");
        assert_eq!(doc["require"]["php"], "^8.2");

        // The path repository is first; the existing one is kept.
        let repos = doc["repositories"].as_array().unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0], path_repository("../api-client"));
        assert_eq!(repos[1]["type"], "composer");

        // Order of keys, at the top and inside sections, is as it was.
        let keys: Vec<&str> = doc
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["name", "require", "require-dev", "repositories", "config"]
        );
        let require: Vec<&str> = doc["require"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(require, ["php", "acme/api-client", "monolog/monolog"]);

        // Four-space indentation and the trailing newline survive.
        assert!(text.contains("\n    \"name\""));
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn requiring_the_package_in_only_one_section_is_enough() {
        let only_dev = r#"{"require": {"php": "^8"}, "require-dev": {"acme/api-client": "^1"}}"#;
        let doc = edited(only_dev, "/abs/api-client");
        assert_eq!(doc["require-dev"]["acme/api-client"], "*");
        assert!(doc["require"].get("acme/api-client").is_none());
        assert_eq!(doc["repositories"][0]["url"], "/abs/api-client");
    }

    #[test]
    fn a_missing_repositories_key_is_created_at_the_end() {
        let doc = edited(
            r#"{"require": {"acme/api-client": "^1"}, "name": "x"}"#,
            "../a",
        );
        let keys: Vec<&str> = doc
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["require", "name", "repositories"]);
    }

    #[test]
    fn object_form_repositories_get_the_overlay_first() {
        let doc = edited(
            r#"{"require": {"acme/api-client": "^1"},
                "repositories": {"packagist.org": false, "private": {"type": "composer", "url": "https://x"}}}"#,
            "../a",
        );
        let repos = doc["repositories"].as_object().unwrap();
        let names: Vec<&str> = repos.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            ["de-overlay-acme/api-client", "packagist.org", "private"]
        );
        assert_eq!(repos["packagist.org"], false);
    }

    #[test]
    fn an_identical_entry_is_not_duplicated() {
        let first = apply_overlay(SAMPLE.as_bytes(), &entries("../api-client")).unwrap();
        let second = apply_overlay(&first, &entries("../api-client")).unwrap();
        assert_eq!(first, second);

        let doc: Value = serde_json::from_slice(&second).unwrap();
        assert_eq!(doc["repositories"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn several_packages_each_get_a_repository() {
        let original = r#"{"require": {"a/one": "^1", "b/two": "~2"}}"#;
        let out = apply_overlay(
            original.as_bytes(),
            &[
                ("a/one".into(), "../one".into()),
                ("b/two".into(), "../two".into()),
            ],
        )
        .unwrap();
        let doc: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(doc["require"]["a/one"], "*");
        assert_eq!(doc["require"]["b/two"], "*");
        assert_eq!(doc["repositories"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn indentation_style_and_missing_final_newline_are_kept() {
        let tabs = "{\n\t\"require\": {\n\t\t\"acme/api-client\": \"^1\"\n\t}\n}";
        let out =
            String::from_utf8(apply_overlay(tabs.as_bytes(), &entries("../a")).unwrap()).unwrap();
        assert!(out.contains("\n\t\"require\""), "{out}");
        assert!(!out.ends_with('\n'));

        let two = "{\n  \"require\": {\"acme/api-client\": \"^1\"}\n}\n";
        let out =
            String::from_utf8(apply_overlay(two.as_bytes(), &entries("../a")).unwrap()).unwrap();
        assert!(out.contains("\n  \"require\""), "{out}");
    }

    #[test]
    fn errors_are_specific() {
        let err = apply_overlay(br#"{"require": {"php": "^8"}}"#, &entries("../a")).unwrap_err();
        assert!(matches!(err, OverlayError::PackageNotRequired(p) if p == "acme/api-client"));

        assert!(matches!(
            apply_overlay(b"{ nope", &entries("../a")).unwrap_err(),
            OverlayError::InvalidComposerJson(_)
        ));
        assert!(matches!(
            apply_overlay(b"[1]", &entries("../a")).unwrap_err(),
            OverlayError::InvalidComposerJson(_)
        ));
        assert!(matches!(
            apply_overlay(&[0xff, 0xfe], &entries("../a")).unwrap_err(),
            OverlayError::InvalidComposerJson(_)
        ));
    }

    fn numbered(text: &str) -> Vec<(Option<u32>, &str)> {
        text.lines()
            .enumerate()
            .map(|(i, l)| (Some(i as u32 + 1), l))
            .collect()
    }

    #[test]
    fn the_scanner_recognises_what_the_overlay_writes() {
        let out = apply_overlay(SAMPLE.as_bytes(), &entries("../api-client")).unwrap();
        let text = String::from_utf8(out).unwrap();
        let findings = scan_composer_json(&numbered(&text), &["acme/api-client"]);

        let kinds: Vec<LeakKind> = findings.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&LeakKind::PathRepository));
        // Both the `require` and the `require-dev` constraint.
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == LeakKind::WildcardConstraint)
                .count(),
            2
        );

        // The untouched file is clean, and so is a wildcard for an unmapped package.
        assert!(scan_composer_json(&numbered(SAMPLE), &["acme/api-client"]).is_empty());
        assert!(
            scan_composer_json(&numbered(&text), &["other/pkg"])
                .iter()
                .all(|f| f.kind == LeakKind::PathRepository)
        );
    }

    #[test]
    fn a_path_repository_without_symlink_is_not_the_overlay() {
        let plain = "\"type\": \"path\",\n\"url\": \"../x\"";
        assert!(scan_composer_json(&numbered(plain), &[]).is_empty());
        let one_line = r#"{"type":"path","url":"../x","options":{"symlink":true}}"#;
        assert_eq!(scan_composer_json(&numbered(one_line), &[]).len(), 1);
    }

    #[test]
    fn escaped_slashes_in_package_names_are_matched() {
        let line = r#""acme\/api-client": "*""#;
        assert_eq!(
            scan_composer_json(&[(Some(1), line)], &["acme/api-client"]).len(),
            1
        );
        // A caret constraint is not the overlay.
        let caret = r#""acme/api-client": "^2""#;
        assert!(scan_composer_json(&[(Some(1), caret)], &["acme/api-client"]).is_empty());
    }

    #[test]
    fn the_lock_scanner_flags_path_dists() {
        let lock = "\"dist\": {\n  \"type\": \"path\",\n  \"url\": \"../api-client\"\n}\n\"dist\": {\"type\": \"zip\"}";
        let findings = scan_composer_lock(&numbered(lock));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, Some(2));
        assert_eq!(findings[0].kind, LeakKind::LockPathDist);
    }
}
