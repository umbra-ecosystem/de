//! Tests for the acli adapter. Fixtures under `providers/fixtures/acli/` have the structure
//! of a real `acli 1.3.39-stable` with invented values (see the README there). The `live_`
//! tests at the bottom run the real installed acli, read-only, and are `#[ignore]`d.

use std::io;
use std::sync::{Arc, Mutex};

use super::*;
use crate::providers::ProviderErrorKind;

const SEARCH: &str = include_str!("../fixtures/acli/search.json");
const VIEW: &str = include_str!("../fixtures/acli/view.json");
const VIEW_COMMENTS: &str = include_str!("../fixtures/acli/view_comments.json");
const COMMENT_LIST_FLAT: &str = include_str!("../fixtures/acli/comment_list_flat.json");

enum Reply {
    Out(Option<i32>, String, String),
    NotFound,
}

fn ok(stdout: &str) -> Reply {
    Reply::Out(Some(0), stdout.into(), String::new())
}

fn fail(code: i32, stderr: &str) -> Reply {
    Reply::Out(Some(code), String::new(), stderr.into())
}

#[derive(Default)]
struct Fake {
    rules: Mutex<Vec<(Vec<String>, Reply)>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl Fake {
    fn on(self, argv: &[&str], reply: Reply) -> Self {
        self.rules
            .lock()
            .unwrap()
            .push((argv.iter().map(|s| String::from(*s)).collect(), reply));
        self
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl CommandRunner for Fake {
    fn run(&self, command: &ExternalCommand) -> eyre::Result<CommandOutput> {
        assert_eq!(command.program, "acli");
        self.calls.lock().unwrap().push(command.args.clone());
        let rules = self.rules.lock().unwrap();
        match rules.iter().find(|(argv, _)| *argv == command.args) {
            Some((_, Reply::Out(code, out, err))) => Ok(CommandOutput {
                success: *code == Some(0),
                code: *code,
                stdout: out.clone(),
                stderr: err.clone(),
            }),
            Some((_, Reply::NotFound)) => Err(eyre::Report::new(io::Error::new(
                io::ErrorKind::NotFound,
                "No such file or directory",
            ))
            .wrap_err("Failed to start `acli`")),
            None => Err(eyre::eyre!("unexpected argv {:?}", command.args)),
        }
    }
}

struct Shared(Arc<Fake>);

impl CommandRunner for Shared {
    fn run(&self, command: &ExternalCommand) -> eyre::Result<CommandOutput> {
        self.0.run(command)
    }
}

fn reader(fake: Fake) -> (AcliJira<Shared>, Arc<Fake>) {
    let fake = Arc::new(fake);
    (
        AcliJira::with_runner(Shared(fake.clone()), Some("example.atlassian.net".into())),
        fake,
    )
}

fn writer(fake: Fake) -> (AcliJiraWriter<Shared>, Arc<Fake>) {
    let fake = Arc::new(fake);
    (
        AcliJiraWriter::with_runner(Shared(fake.clone()), None),
        fake,
    )
}

fn key(s: &str) -> TicketKey {
    s.parse().unwrap()
}

fn search_argv(jql: &str) -> Vec<&str> {
    vec![
        "jira",
        "workitem",
        "search",
        "--jql",
        jql,
        "--paginate",
        "--json",
        "--fields",
        SEARCH_FIELDS,
    ]
}

fn view_argv<'a>(k: &'a str, fields: &'a str) -> Vec<&'a str> {
    vec!["jira", "workitem", "view", k, "--json", "--fields", fields]
}

/// What the real acli prints for a missing key (`view`): stderr only, exit 1.
const MISSING_ERR: &str =
    "✗ Error: Issue does not exist or you do not have permission to see it.\n";

fn assert_send<T: Send>() {}

#[test]
fn adapters_are_send_and_construction_does_not_spawn() {
    assert_send::<AcliJira>();
    assert_send::<AcliJiraWriter>();
    let config = Config::default();
    AcliJira::new(&config).unwrap();
    AcliJiraWriter::new(&config).unwrap();
}

// ---- health ----------------------------------------------------------------------------

#[test]
fn health_ready() {
    let (jira, _) = reader(
        Fake::default()
            .on(&["--version"], ok("acli version 1.3.39-stable\n"))
            .on(
                &["jira", "auth", "status"],
                ok("✓ Authenticated\n  Site: example.atlassian.net\n  Email: jane.doe@example.com\n  Authentication Type: api_token\n"),
            ),
    );
    let h = jira.health();
    assert!(h.is_ready(), "{h:?}");
    assert_eq!(h.version.as_deref(), Some("1.3.39"));
}

#[test]
fn health_not_installed() {
    let (jira, fake) = reader(Fake::default().on(&["--version"], Reply::NotFound));
    let h = jira.health();
    assert!(!h.installed && !h.is_ready());
    assert_eq!(fake.calls().len(), 1);
}

#[test]
fn health_not_authenticated_by_exit_code_and_by_wording() {
    let (jira, _) = reader(
        Fake::default()
            .on(&["--version"], ok("1.3.4"))
            .on(&["jira", "auth", "status"], fail(1, "not logged in")),
    );
    let h = jira.health();
    assert!(h.installed && !h.authenticated && h.meets_minimum, "{h:?}");

    let (jira, _) = reader(
        Fake::default()
            .on(&["--version"], ok("1.3.4"))
            .on(&["jira", "auth", "status"], ok("You are not logged in\n")),
    );
    assert!(!jira.health().authenticated);
}

#[test]
fn health_version_below_minimum_or_unreadable() {
    let (jira, _) = reader(
        Fake::default()
            .on(&["--version"], ok("acli version 1.2.9"))
            .on(&["jira", "auth", "status"], ok("ok")),
    );
    let h = jira.health();
    assert!(
        h.authenticated && !h.meets_minimum && !h.is_ready(),
        "{h:?}"
    );

    let (jira, _) = reader(
        Fake::default()
            .on(&["--version"], ok("garbage"))
            .on(&["jira", "auth", "status"], ok("ok")),
    );
    let h = jira.health();
    assert!(h.version.is_none() && !h.meets_minimum);

    let (jira, _) = reader(Fake::default().on(&["--version"], fail(2, "unknown flag")));
    assert!(!jira.health().is_ready());
}

#[test]
fn version_parsing() {
    assert_eq!(
        parse_version("acli version 1.3.15-stable"),
        Some((1, 3, 15))
    );
    assert_eq!(parse_version("v2.0"), Some((2, 0, 0)));
    // The real output: `acli version 1.3.39-stable`.
    assert_eq!(
        parse_version("acli version 1.3.39-stable\n"),
        Some((1, 3, 39))
    );
    assert!(
        parse_version("acli version 1.3.39-stable").unwrap()
            >= parse_version(MIN_ACLI_VERSION).unwrap()
    );
    assert_eq!(parse_version("nothing"), None);
    assert!(parse_version(MIN_ACLI_VERSION).is_some());
}

// ---- reads -----------------------------------------------------------------------------

#[test]
fn search_combines_pages_and_maps_fields() {
    let (jira, fake) = reader(Fake::default().on(&search_argv("project = PROJ"), ok(SEARCH)));
    let found = jira.search("project = PROJ").unwrap();
    let keys: Vec<&str> = found.iter().map(|t| t.key.as_str()).collect();
    assert_eq!(keys, ["PROJ-123", "PROJ-124", "PROJ-125"]);
    let first = &found[0];
    assert_eq!(first.title, "Fix login redirect");
    assert_eq!(first.status, "In Review");
    assert_eq!(first.priority.as_deref(), Some("High"));
    assert_eq!(first.assignee.as_deref(), Some("Jane Doe"));
    assert_eq!(
        first.url.as_deref(),
        Some("https://example.atlassian.net/browse/PROJ-123")
    );
    assert_eq!(found[1].assignee, None);
    assert_eq!(found[2].status, "Done");
    assert_eq!(found[2].priority, None);
    // Search cannot return `updated`; every result carries the documented sentinel.
    assert!(found.iter().all(|t| t.updated_at == UPDATED_UNKNOWN));
    // The JQL is one verbatim argv element.
    assert_eq!(fake.calls().len(), 1);
}

#[test]
fn every_acli_call_is_written_to_the_run_log() {
    let _serial = crate::synclog::TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let (jira, _fake) = reader(Fake::default().on(&search_argv("project = PROJ"), ok(SEARCH)));

    let run = crate::synclog::begin(dir.path(), 1_700_000_000, 5).unwrap();
    jira.search("project = PROJ").unwrap();
    drop(run);

    let files = crate::synclog::list(dir.path()).unwrap();
    let text = crate::synclog::read(dir.path(), &files[0].name).unwrap();
    assert!(text.contains("$ acli jira workitem search --jql project = PROJ"), "{text}");
    assert!(text.contains("exit 0 in"), "{text}");
    assert!(text.contains("Fix login redirect"), "raw stdout is kept: {text}");
}

/// A synthetic `view --fields '*all'` output with the structure of a real one: invented values throughout.
const VIEW_ALL: &str = r#"{
  "key": "PROJ-7",
  "fields": {
    "summary": "Synthetic ticket",
    "issuetype": {"name": "Story"},
    "reporter": {"accountId": "acc-r", "displayName": "Riley Reporter"},
    "created": "2026-09-01T08:30:00.000+0000",
    "updated": "2026-09-02T09:45:10.000+0000",
    "labels": ["web", "reports"],
    "components": [{"name": "Frontend"}],
    "fixVersions": [{"name": "2.14.0"}],
    "timetracking": {"originalEstimate": "3d"},
    "customfield_10018": [
      {"boardId": 1, "id": 11, "name": "Sprint 40", "state": "closed"},
      {"boardId": 1, "id": 12, "name": "Sprint 41", "state": "active"}
    ],
    "customfield_10019": "0|i00abc:",
    "parent": {"key": "PROJ-1", "fields": {"summary": "Reporting epic"}},
    "description": {"type": "doc", "version": 1, "content": [
      {"type": "paragraph", "content": [{"type": "text", "text": "First paragraph."}]},
      {"type": "bulletList", "content": [
        {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "one"}]}]},
        {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "two"}]}]}
      ]}
    ]},
    "attachment": [{"filename": "spec.pdf", "size": 2048}],
    "issuelinks": [
      {"type": {"inward": "is blocked by", "outward": "blocks"},
       "outwardIssue": {"key": "PROJ-8", "fields": {"summary": "Downstream", "status": {"name": "To Do"}}}},
      {"type": {"inward": "is blocked by", "outward": "blocks"},
       "inwardIssue": {"key": "PROJ-2", "fields": {"summary": "Upstream", "status": {"name": "Done"}}}}
    ],
    "subtasks": [
      {"key": "PROJ-9", "fields": {"summary": "Write tests", "status": {"statusCategory": {"key": "done"}}}},
      {"key": "PROJ-10", "fields": {"summary": "Ship it", "status": {"statusCategory": {"key": "new"}}}}
    ],
    "comment": {"total": 0, "comments": []}
  }
}"#;

#[test]
fn view_all_reads_the_detail_and_the_comments_in_one_output() {
    let (detail, comments, total) = wire::parse_view_all(VIEW_ALL, &key("PROJ-7")).unwrap();
    assert!(comments.is_empty() && total == Some(0));
    assert_eq!(detail.issue_type.as_deref(), Some("Story"));
    assert_eq!(detail.reporter.as_deref(), Some("Riley Reporter"));
    assert_eq!(detail.created_at, crate::providers::acli::time::parse_timestamp("2026-09-01T08:30:00.000+0000"));
    assert!(detail.updated_at > detail.created_at);
    assert_eq!(detail.labels, ["web", "reports"]);
    assert_eq!(detail.components, ["Frontend"]);
    assert_eq!(detail.fix_versions, ["2.14.0"]);
    assert_eq!(detail.original_estimate.as_deref(), Some("3d"));
    assert_eq!(detail.sprint.as_deref(), Some("Sprint 41"), "found by shape, the active one");
    assert_eq!(detail.parent, Some(("PROJ-1".into(), "Reporting epic".into())));
    assert_eq!(detail.description, "First paragraph.\n\n- one\n- two");
    assert_eq!(detail.attachments[0].name, "spec.pdf");
    assert_eq!(detail.attachments[0].bytes, 2048);
    assert_eq!(detail.links.len(), 2);
    assert_eq!((detail.links[0].relation.as_str(), detail.links[0].key.as_str()), ("blocks", "PROJ-8"));
    assert_eq!((detail.links[1].relation.as_str(), detail.links[1].status.as_str()), ("is blocked by", "Done"));
    assert_eq!(detail.subtasks.iter().map(|s| s.done).collect::<Vec<_>>(), [true, false]);
}

#[test]
fn view_all_with_a_bare_ticket_gives_an_empty_detail_not_an_error() {
    let bare = r#"{"key":"PROJ-7","fields":{"summary":"x","description":null,"labels":[],"comment":{"total":0,"comments":[]}}}"#;
    let (detail, comments, _) = wire::parse_view_all(bare, &key("PROJ-7")).unwrap();
    assert!(comments.is_empty());
    assert_eq!(detail, crate::providers::TicketDetail::default());
}

#[test]
fn view_asks_for_all_fields_in_a_single_call() {
    let argv = ["jira", "workitem", "view", "PROJ-7", "--json", "--fields", "*all"];
    let (jira, fake) = reader(Fake::default().on(&argv, ok(VIEW_ALL)));
    let viewed = jira.view(&key("PROJ-7")).unwrap();
    assert_eq!(viewed.detail.unwrap().sprint.as_deref(), Some("Sprint 41"));
    assert_eq!(fake.calls().len(), 1);
}

#[test]
fn search_never_asks_for_updated_which_real_acli_rejects() {
    // `--fields ...,updated` makes the real acli exit 1: "field 'updated' is not allowed".
    let fields: Vec<&str> = SEARCH_FIELDS.split(',').collect();
    assert!(!fields.contains(&"updated"));
    // `--fields key` alone yields an array of nulls, so summary is always requested.
    assert!(fields.contains(&"summary") && fields.contains(&"key"));
    let (jira, fake) = reader(Fake::default().on(&search_argv("q"), ok("[]")));
    jira.search("q").unwrap();
    let call = fake.calls().remove(0);
    assert!(!call.iter().any(|a| a.contains("updated")), "{call:?}");
    // And the failure the real tool gives for it is an error, not empty results.
    let (jira, _) = reader(Fake::default().on(
        &search_argv("q"),
        fail(1, "✗ Error: field 'updated' is not allowed\n"),
    ));
    assert!(jira.search("q").is_err());
}

#[test]
fn stdout_errors_and_error_marker_with_exit_zero_are_errors() {
    // Some acli failures print the message on stdout; a generic line goes to stderr.
    let (jira, _) = reader(Fake::default().on(
        &search_argv("q"),
        Reply::Out(
            Some(1),
            "✗ Error: Issue does not exist or you do not have permission to see it.\n".into(),
            "✗ Error: command execution failed\n".into(),
        ),
    ));
    let e = jira.search("q").unwrap_err();
    assert_eq!(e.kind(), ProviderErrorKind::NotFound);
    assert!(e.to_string().contains("does not exist"), "{e}");
    // Exit 0 but an error marker on stdout is still an error, never a parse of prose.
    let (jira, _) = reader(Fake::default().on(
        &search_argv("q"),
        ok("✗ Error: Unbounded JQL queries are not allowed here.\n"),
    ));
    assert_eq!(
        jira.search("q").unwrap_err().kind(),
        ProviderErrorKind::Command
    );
}

#[test]
fn search_accepts_wrapped_objects_and_an_empty_array_but_not_empty_output() {
    let wrapped = r#"{"issues":[{"key":"APP-9","fields":{"summary":"s","status":{"name":"Open"}}}],"total":1}"#;
    let (jira, _) = reader(
        Fake::default()
            .on(&search_argv("a"), ok(wrapped))
            .on(&search_argv("b"), ok("[]"))
            .on(&search_argv("c"), ok("")),
    );
    let t = jira.search("a").unwrap();
    assert_eq!((t.len(), t[0].updated_at), (1, 0));
    assert!(jira.search("b").unwrap().is_empty());
    // Verified against a real acli: a search with no result prints `[]`. Nothing at all on
    // stdout (exit 0) is a run that did not answer, and reading it as "no tickets" would make
    // sync delete every cached ticket and comment that is not tracked.
    let e = jira.search("c").unwrap_err();
    assert_eq!(e.kind(), ProviderErrorKind::Parse, "{e}");
}

#[test]
fn a_ticket_number_that_looks_like_an_http_status_does_not_change_the_error_kind() {
    // `PROJ-401` and `PROJ-404` are tickets, not HTTP statuses: a ticket that cannot be
    // read must stay a per-ticket NotFound and must not abort the whole sync as "not
    // logged in".
    let key401 = key("PROJ-401");
    let argv = [
        "jira",
        "workitem",
        "view",
        "PROJ-401",
        "--json",
        "--fields",
        VIEW_FIELDS,
    ];
    let (jira, _) = reader(Fake::default().on(
        &argv,
        fail(
            1,
            "✗ Error: Issue PROJ-401 does not exist or you do not have permission to see it.\n",
        ),
    ));
    assert_eq!(
        jira.get(&key401).unwrap_err().kind(),
        ProviderErrorKind::NotFound
    );

    // A real status still counts.
    let (jira, _) = reader(Fake::default().on(&argv, fail(1, "✗ Error: HTTP 401 Unauthorized\n")));
    assert_eq!(
        jira.get(&key("PROJ-401")).unwrap_err().kind(),
        ProviderErrorKind::NotAuthenticated
    );
    let (jira, _) =
        reader(Fake::default().on(&argv, fail(1, "✗ Error: request failed with status 401\n")));
    assert_eq!(
        jira.get(&key("PROJ-401")).unwrap_err().kind(),
        ProviderErrorKind::NotAuthenticated
    );
}

#[test]
fn malformed_output_is_a_parse_error_with_a_truncated_snippet() {
    let junk = format!("<html>{}", "x".repeat(2000));
    let (jira, _) = reader(Fake::default().on(&search_argv("q"), ok(&junk)));
    let err = jira.search("q").unwrap_err();
    assert_eq!(err.kind(), ProviderErrorKind::Parse);
    let text = err.to_string();
    assert!(text.contains("acli") && text.contains("search results") && text.contains("<html>"));
    assert!(text.len() < 700, "{}", text.len());

    let (jira, _) = reader(Fake::default().on(&search_argv("q"), ok(r#"[{"key":"nope"}]"#)));
    assert_eq!(
        jira.search("q").unwrap_err().kind(),
        ProviderErrorKind::Parse
    );

    let (jira, _) = reader(Fake::default().on(&search_argv("q"), ok(r#"{"a":1}"#)));
    assert_eq!(
        jira.search("q").unwrap_err().kind(),
        ProviderErrorKind::Parse
    );
}

#[test]
fn get_maps_one_ticket_with_updated_and_reports_missing() {
    let view = |k: &'static str| view_argv(k, VIEW_FIELDS);
    let jira = AcliJira::with_runner(
        Shared(Arc::new(
            Fake::default()
                .on(&view("PROJ-123"), ok(VIEW))
                .on(&view("PROJ-2"), fail(1, MISSING_ERR))
                .on(&view("PROJ-3"), fail(1, "401 Unauthorized"))
                .on(
                    &view("PROJ-4"),
                    fail(1, "dial tcp: lookup example.atlassian.net: no such host"),
                )
                .on(&view("PROJ-5"), ok("[]")),
        )),
        None,
    );
    let t = jira.get(&key("PROJ-123")).unwrap();
    assert_eq!(
        (t.status.as_str(), t.title.as_str()),
        ("UAT", "Fix login redirect")
    );
    // `updated` is only available from view: 2026-01-15T09:30:45Z.
    assert_eq!(t.updated_at, 1_768_469_445);
    // No site configured: the URL is derived from `self`.
    assert_eq!(
        t.url.as_deref(),
        Some("https://example.atlassian.net/browse/PROJ-123")
    );
    // The real acli says this on stderr (exit 1) for a missing or invisible key.
    assert_eq!(
        jira.get(&key("PROJ-2")).unwrap_err().kind(),
        ProviderErrorKind::NotFound
    );
    assert_eq!(
        jira.get(&key("PROJ-3")).unwrap_err().kind(),
        ProviderErrorKind::NotAuthenticated
    );
    assert_eq!(
        jira.get(&key("PROJ-4")).unwrap_err().kind(),
        ProviderErrorKind::Network
    );
    assert_eq!(
        jira.get(&key("PROJ-5")).unwrap_err().kind(),
        ProviderErrorKind::NotFound
    );
}

#[test]
fn view_requests_updated_and_search_does_not() {
    assert!(VIEW_FIELDS.split(',').any(|f| f == "updated"));
    assert!(!SEARCH_FIELDS.split(',').any(|f| f == "updated"));
}

// ---- comments come from `view --fields comment` ------------------------------------------

fn comments_argv(k: &str) -> Vec<&str> {
    view_argv(k, COMMENT_FIELDS)
}

#[test]
fn comments_come_from_view_fields_comment_oldest_first_with_mentions() {
    let (jira, fake) = reader(Fake::default().on(&comments_argv("PROJ-123"), ok(VIEW_COMMENTS)));
    let list = jira.comments(&key("PROJ-123")).unwrap();
    assert_eq!(
        fake.calls(),
        vec![
            [
                "jira", "workitem", "view", "PROJ-123", "--json", "--fields", "comment"
            ]
            .map(String::from)
            .to_vec()
        ]
    );
    assert_eq!(list.len(), 2);
    // Fixture order is newest first; the adapter returns oldest first.
    assert_eq!(list[0].id, "30001");
    assert_eq!(list[0].author_account_id, "acct-0001");
    assert_eq!(list[0].author_name, "Jane Doe");
    assert_eq!(list[0].body_text, "Ready for review.");
    assert!(list[0].mentions.is_empty());
    assert_eq!(list[0].created_at, 1_768_471_200); // 2026-01-15T10:00:00Z
    assert_eq!(list[0].ticket, key("PROJ-123"));
    assert_eq!(list[1].id, "30002");
    assert_eq!(list[1].author_account_id, "acct-0002");
    assert_eq!(list[1].author_name, "John Roe");
    // 2026-01-16T14:05:00+01:00 = 13:05:00Z
    assert_eq!(list[1].created_at, 1_768_568_700);
    assert!(list[0].created_at < list[1].created_at);
    // Mentions come from ADF `mention` nodes: deduplicated, in order of appearance.
    assert_eq!(list[1].mentions, ["acct-0001", "acct-0003"]);
    // No warning: total matches what was returned.
    assert!(jira.take_warnings().is_empty());
}

#[test]
fn adf_with_unknown_and_media_node_types_never_fails_parsing() {
    let (jira, _) = reader(Fake::default().on(&comments_argv("PROJ-123"), ok(VIEW_COMMENTS)));
    let list = jira.comments(&key("PROJ-123")).unwrap();
    let text = &list[1].body_text;
    assert!(
        text.starts_with("@Jane Doe could you re-check this?"),
        "{text}"
    );
    assert!(text.contains("Thanks"), "{text}");
    assert!(text.contains("- first point"), "{text}");
    assert!(text.contains("PROJ-999"), "{text}");
    assert!(text.contains("cell"), "{text}");
    // The invented `futureWidget` node is rendered from its children, not rejected.
    assert!(text.contains("inside unknown"), "{text}");
}

#[test]
fn comment_list_flat_shape_is_not_used_and_not_accepted() {
    // The flat `comment list` output has no account ids, timestamps or mention data, so
    // reads must never run it ...
    let (jira, fake) = reader(
        Fake::default()
            .on(&comments_argv("PROJ-123"), ok(VIEW_COMMENTS))
            .on(&search_argv("q"), ok(SEARCH))
            .on(&view_argv("PROJ-123", VIEW_FIELDS), ok(VIEW)),
    );
    jira.comments(&key("PROJ-123")).unwrap();
    jira.search("q").unwrap();
    jira.get(&key("PROJ-123")).unwrap();
    for call in fake.calls() {
        assert!(!call.join(" ").contains("comment list"), "{call:?}");
    }
    // ... and if that shape ever shows up where comments are expected it is a parse
    // error, not "no comments".
    let (jira, _) = reader(Fake::default().on(&comments_argv("PROJ-123"), ok(COMMENT_LIST_FLAT)));
    assert_eq!(
        jira.comments(&key("PROJ-123")).unwrap_err().kind(),
        ProviderErrorKind::Parse
    );
}

#[test]
fn truncated_comment_lists_are_reported_through_take_warnings() {
    // total says 40, the container returned 2.
    let truncated = VIEW_COMMENTS.replace(r#""total": 2"#, r#""total": 40"#);
    let (jira, _) = reader(Fake::default().on(&comments_argv("PROJ-123"), ok(&truncated)));
    let list = jira.comments(&key("PROJ-123")).unwrap();
    assert_eq!(list.len(), 2);
    let warnings = jira.take_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("PROJ-123") && warnings[0].contains("2 of 40"),
        "{warnings:?}"
    );
    // Drained.
    assert!(jira.take_warnings().is_empty());
}

#[test]
fn comments_null_empty_broken_and_missing_key() {
    let none = r#"{"key":"PROJ-1","fields":{"comment":{"comments":[],"total":0,"maxResults":50,"startAt":0}}}"#;
    let nulled = r#"{"key":"PROJ-2","fields":{"comment":null}}"#;
    let no_id = r#"{"fields":{"comment":{"comments":[{"author":{}}],"total":1}}}"#;
    let (jira, _) = reader(
        Fake::default()
            .on(&comments_argv("PROJ-1"), ok(none))
            .on(&comments_argv("PROJ-2"), ok(nulled))
            .on(&comments_argv("PROJ-3"), ok(no_id))
            .on(&comments_argv("PROJ-4"), ok("not json"))
            .on(&comments_argv("PROJ-5"), fail(1, MISSING_ERR)),
    );
    assert!(jira.comments(&key("PROJ-1")).unwrap().is_empty());
    assert!(jira.comments(&key("PROJ-2")).unwrap().is_empty());
    assert_eq!(
        jira.comments(&key("PROJ-3")).unwrap_err().kind(),
        ProviderErrorKind::Parse
    );
    assert_eq!(
        jira.comments(&key("PROJ-4")).unwrap_err().kind(),
        ProviderErrorKind::Parse
    );
    assert_eq!(
        jira.comments(&key("PROJ-5")).unwrap_err().kind(),
        ProviderErrorKind::NotFound
    );
}

#[test]
fn reads_never_issue_write_commands() {
    let (jira, fake) = reader(
        Fake::default()
            .on(&["--version"], ok("1.3.4"))
            .on(&["jira", "auth", "status"], ok("ok"))
            .on(&search_argv("q"), ok("[]"))
            .on(
                &comments_argv("APP-1"),
                ok(r#"{"fields":{"comment":null}}"#),
            ),
    );
    jira.health();
    jira.search("q").unwrap();
    jira.comments(&key("APP-1")).unwrap();
    let _ = jira.get(&key("APP-1"));
    for call in fake.calls() {
        let joined = call.join(" ");
        assert!(
            !joined.contains("create")
                && !joined.contains("transition")
                && !joined.contains("edit"),
            "{joined}"
        );
    }
}

#[test]
fn a_missing_binary_is_not_installed_for_every_method() {
    let (jira, _) = reader(Fake::default().on(&search_argv("q"), Reply::NotFound));
    let e = jira.search("q").unwrap_err();
    assert_eq!(e.kind(), ProviderErrorKind::NotInstalled);
    assert!(e.is_environmental());
}

// ---- writes ----------------------------------------------------------------------------

#[test]
fn comment_is_posted_verbatim_as_one_argument() {
    let body = "line one\n  \"quoted\" and 'single' $HOME `x` ünï — 日本 --json\n";
    let argv = [
        "jira", "workitem", "comment", "create", "--key", "APP-1", "--body", body, "--json",
    ];
    let reply = r#"{"id":"30001","author":{"accountId":"me","displayName":"Me"},"created":"2024-05-01T10:15:30.000+0000","body":"stored"}"#;
    let (w, fake) = writer(Fake::default().on(&argv, ok(reply)));
    let c = w.add_comment(&key("APP-1"), body).unwrap();
    assert_eq!(
        (c.id.as_str(), c.author_account_id.as_str()),
        ("30001", "me")
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][7], body);
}

#[test]
fn comment_survives_an_unreadable_reply_without_a_duplicate_post() {
    let argv = [
        "jira",
        "workitem",
        "comment",
        "create",
        "--key",
        "APP-1",
        "--body",
        "hi [~accountid:a1]",
        "--json",
    ];
    let (w, fake) = writer(Fake::default().on(&argv, ok("Comment created")));
    let c = w.add_comment(&key("APP-1"), "hi [~accountid:a1]").unwrap();
    assert_eq!(c.body_text, "hi [~accountid:a1]");
    assert_eq!(c.mentions, ["a1"]);
    assert_eq!(fake.calls().len(), 1);
}

#[test]
fn comment_refuses_empty_and_adf_bodies_without_running_acli() {
    let (w, fake) = writer(Fake::default());
    for body in ["", "  \n", r#"{"type":"doc","version":1,"content":[]}"#] {
        let e = w.add_comment(&key("APP-1"), body).unwrap_err();
        assert_eq!(e.kind(), ProviderErrorKind::Unsupported, "{body}");
    }
    assert!(fake.calls().is_empty());
    // JSON that is not ADF is an ordinary comment.
    let (w, fake) = writer(Fake::default().on(
        &[
            "jira",
            "workitem",
            "comment",
            "create",
            "--key",
            "APP-1",
            "--body",
            r#"{"a":1}"#,
            "--json",
        ],
        ok(""),
    ));
    w.add_comment(&key("APP-1"), r#"{"a":1}"#).unwrap();
    assert_eq!(fake.calls().len(), 1);
}

fn transition_argv(status: &str) -> Vec<&str> {
    vec![
        "jira",
        "workitem",
        "transition",
        "--key",
        "APP-1",
        "--status",
        status,
        "--yes",
        "--json",
    ]
}

#[test]
fn transition_succeeds_and_passes_the_status_verbatim() {
    let (w, fake) = writer(Fake::default().on(&transition_argv("Alpha Testing"), ok("[]")));
    w.transition(&key("APP-1"), "Alpha Testing").unwrap();
    assert_eq!(fake.calls()[0][6], "Alpha Testing");
}

#[test]
fn transition_to_an_unreachable_status_is_unsupported_and_quotes_acli() {
    let (w, _) = writer(
        Fake::default()
            .on(
                &transition_argv("Nope"),
                fail(
                    1,
                    "Error: status \"Nope\" is invalid. Valid: To Do, In Progress, Done",
                ),
            )
            .on(
                &transition_argv("Soft"),
                ok(r#"[{"key":"APP-1","success":false,"error":"no transition"}]"#),
            )
            .on(&transition_argv("Auth"), fail(1, "not logged in")),
    );
    let e = w.transition(&key("APP-1"), "Nope").unwrap_err();
    assert_eq!(e.kind(), ProviderErrorKind::Unsupported);
    let text = e.to_string();
    assert!(
        text.contains("To Do, In Progress, Done") && text.contains("Nope"),
        "{text}"
    );
    assert_eq!(
        w.transition(&key("APP-1"), "Soft").unwrap_err().kind(),
        ProviderErrorKind::Unsupported
    );
    assert_eq!(
        w.transition(&key("APP-1"), "Auth").unwrap_err().kind(),
        ProviderErrorKind::NotAuthenticated
    );
    assert_eq!(
        w.transition(&key("APP-1"), " ").unwrap_err().kind(),
        ProviderErrorKind::Unsupported
    );
}

// ---- probe -----------------------------------------------------------------------------

#[test]
fn probe_runs_only_read_only_commands_and_never_fails() {
    let fake = Fake::default().on(&["--version"], ok("1.3.4"));
    let results = probe(&fake, Some(&key("APP-1")));
    assert_eq!(results.len(), 5);
    assert_eq!(results[0].stdout, "1.3.4");
    assert_eq!(results[0].exit_code, Some(0));
    // Unscripted commands come back as results, not panics.
    assert_eq!(results[1].exit_code, None);
    assert!(results[1].stderr.contains("could not run"));
    assert!(results[2].command.contains("--limit 1"));
    // The search is bounded (acli rejects a bare `order by`) and never asks for `updated`.
    assert!(results[2].command.contains("currentUser()"));
    assert!(
        !results[2]
            .command
            .contains("key,summary,status,priority,assignee,updated")
    );
    // The last probe reads the comment field, not the flat `comment list`.
    assert!(
        results[4].command.ends_with("--fields comment"),
        "{}",
        results[4].command
    );
    for call in fake.calls() {
        let joined = call.join(" ");
        assert!(
            !joined.contains("create")
                && !joined.contains("transition")
                && !joined.contains("comment list"),
            "{joined}"
        );
    }
    assert_eq!(probe(&Fake::default(), None).len(), 3);
}

#[test]
fn a_hung_acli_is_an_environmental_error_and_is_killed() {
    use std::os::unix::fs::PermissionsExt;
    let bin = tempfile::tempdir().unwrap();
    let stub = bin.path().join("acli");
    std::fs::write(&stub, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let runner = ProcessRunner::new()
        .with_path_prefix(bin.path())
        .with_timeout(Duration::from_millis(200));
    let jira = AcliJira::with_runner(runner, None);

    let started = std::time::Instant::now();
    let err = jira.search("assignee = currentUser()").unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(err.kind(), ProviderErrorKind::Network, "{err}");
    assert!(err.is_environmental());
    assert!(err.to_string().contains("timed out"), "{err}");
}

// ---- live (opt-in): cargo test -p de-core -- --ignored live_ ---------------------------
//
// These run the real installed `acli`, read-only, against whatever Jira it is logged in to.
// They print nothing about the data (only counts and lengths).

fn live() -> AcliJira {
    AcliJira::new(&Config::default()).unwrap()
}

#[test]
#[ignore = "runs the real installed acli (read-only)"]
fn live_health_is_ready() {
    let h = live().health();
    assert!(h.installed, "acli is not installed");
    assert!(
        h.authenticated && h.meets_minimum && h.is_ready(),
        "{}",
        h.detail
    );
    assert!(h.version.is_some());
}

#[test]
#[ignore = "runs the real installed acli (read-only)"]
fn live_search_get_and_comments_parse() {
    let jira = live();
    let found = jira
        .search("assignee = currentUser() order by updated DESC")
        .map(|mut v| {
            v.truncate(1);
            v
        });
    // `search` returns everything; that is fine for the assignee-bounded query above.
    let found = found.unwrap_or_else(|e| panic!("search failed: {:?}", e.kind()));
    assert!(
        !found.is_empty(),
        "the logged-in user has no assigned tickets"
    );
    let t = &found[0];
    assert!(!t.key.as_str().is_empty() && !t.title.is_empty() && !t.status.is_empty());
    assert_eq!(t.updated_at, UPDATED_UNKNOWN);

    let got = jira
        .get(&t.key)
        .unwrap_or_else(|e| panic!("get failed: {:?}", e.kind()));
    assert_eq!(got.key, t.key);
    assert!(got.updated_at > 0, "view should return `updated`");
    assert!(!got.title.is_empty() && !got.status.is_empty());

    let comments = jira
        .comments(&t.key)
        .unwrap_or_else(|e| panic!("comments failed: {:?}", e.kind()));
    for c in &comments {
        assert!(!c.id.is_empty() && c.created_at > 0);
        assert!(!c.author_account_id.is_empty());
    }
    let mentions: usize = comments.iter().map(|c| c.mentions.len()).sum();
    eprintln!(
        "live: {} tickets, {} comments, {} mentions, warnings {}",
        found.len(),
        comments.len(),
        mentions,
        jira.take_warnings().len()
    );
}

#[test]
#[ignore = "runs the real installed acli (read-only)"]
fn live_missing_key_is_not_found() {
    let e = live().get(&key("ZZZ-999999")).unwrap_err();
    assert_eq!(e.kind(), ProviderErrorKind::NotFound);
}
