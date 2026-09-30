//! Tests for the acli adapter. Fixtures under `providers/fixtures/acli/` are derived from
//! documentation, NOT captured from a real install.

use std::io;
use std::sync::{Arc, Mutex};

use super::*;
use crate::providers::ProviderErrorKind;

const PAGE1: &str = include_str!("../fixtures/acli/search_page1.json");
const PAGE2: &str = include_str!("../fixtures/acli/search_page2.json");
const COMMENTS: &str = include_str!("../fixtures/acli/comments.json");

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
        AcliJira::with_runner(Shared(fake.clone()), Some("acme.atlassian.net".into())),
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
        TICKET_FIELDS,
    ]
}

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
            .on(&["--version"], ok("acli version 1.3.15-stable\n"))
            .on(
                &["jira", "auth", "status"],
                ok("Logged in as ada@acme.test\n"),
            ),
    );
    let h = jira.health();
    assert!(h.is_ready(), "{h:?}");
    assert_eq!(h.version.as_deref(), Some("1.3.15"));
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
    assert_eq!(parse_version("nothing"), None);
    assert!(parse_version(MIN_ACLI_VERSION).is_some());
}

// ---- reads -----------------------------------------------------------------------------

#[test]
fn search_combines_pages_and_maps_fields() {
    let two_docs = format!("{PAGE1}\n{PAGE2}");
    let (jira, fake) = reader(Fake::default().on(&search_argv("project = APP"), ok(&two_docs)));
    let found = jira.search("project = APP").unwrap();
    let keys: Vec<&str> = found.iter().map(|t| t.key.as_str()).collect();
    assert_eq!(keys, ["APP-1", "APP-2", "APP-3"]);
    let first = &found[0];
    assert_eq!(first.title, "Fix login");
    assert_eq!(first.status, "In Review");
    assert_eq!(first.priority.as_deref(), Some("High"));
    assert_eq!(first.assignee.as_deref(), Some("Ada Lovelace"));
    assert_eq!(
        first.url.as_deref(),
        Some("https://acme.atlassian.net/browse/APP-1")
    );
    assert_eq!(first.updated_at, 1_714_558_530);
    assert_eq!(found[1].assignee, None);
    assert_eq!(found[1].updated_at, 1_714_600_800);
    assert_eq!(found[2].status, "Done");
    // The JQL is one verbatim argv element.
    assert_eq!(fake.calls().len(), 1);
}

#[test]
fn search_accepts_wrapped_objects_and_empty_output() {
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
    assert!(jira.search("c").unwrap().is_empty());
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
fn get_maps_one_ticket_and_reports_missing() {
    let view = |k: &'static str| {
        vec![
            "jira",
            "workitem",
            "view",
            k,
            "--json",
            "--fields",
            TICKET_FIELDS,
        ]
    };
    let one = r#"{"key":"APP-1","self":"https://acme.atlassian.net/rest/api/3/issue/1","fields":{"summary":"S","status":{"name":"UAT"}}}"#;
    let jira = AcliJira::with_runner(
        Shared(Arc::new(
            Fake::default()
                .on(&view("APP-1"), ok(one))
                .on(
                    &view("APP-2"),
                    fail(
                        1,
                        "Error: issue does not exist or you do not have permission",
                    ),
                )
                .on(&view("APP-3"), fail(1, "401 Unauthorized"))
                .on(
                    &view("APP-4"),
                    fail(1, "dial tcp: lookup acme.atlassian.net: no such host"),
                )
                .on(&view("APP-5"), ok("[]")),
        )),
        None,
    );
    let t = jira.get(&key("APP-1")).unwrap();
    assert_eq!((t.status.as_str(), t.title.as_str()), ("UAT", "S"));
    // No site configured: the URL is derived from `self`.
    assert_eq!(
        t.url.as_deref(),
        Some("https://acme.atlassian.net/browse/APP-1")
    );
    assert_eq!(
        jira.get(&key("APP-2")).unwrap_err().kind(),
        ProviderErrorKind::NotFound
    );
    assert_eq!(
        jira.get(&key("APP-3")).unwrap_err().kind(),
        ProviderErrorKind::NotAuthenticated
    );
    assert_eq!(
        jira.get(&key("APP-4")).unwrap_err().kind(),
        ProviderErrorKind::Network
    );
    assert_eq!(
        jira.get(&key("APP-5")).unwrap_err().kind(),
        ProviderErrorKind::NotFound
    );
}

fn comments_argv(k: &str) -> Vec<&str> {
    vec![
        "jira",
        "workitem",
        "comment",
        "list",
        "--key",
        k,
        "--paginate",
        "--json",
    ]
}

#[test]
fn comments_are_oldest_first_with_mentions() {
    let (jira, _) = reader(Fake::default().on(&comments_argv("APP-1"), ok(COMMENTS)));
    let list = jira.comments(&key("APP-1")).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, "20001");
    assert_eq!(list[0].mentions, ["acc-3"]);
    assert_eq!(list[0].body_text, "Plain text with @acc-3 mention");
    assert_eq!(list[1].id, "20002");
    assert_eq!(list[1].author_account_id, "acc-2");
    assert_eq!(list[1].author_name, "Grace Hopper");
    assert_eq!(list[1].mentions, ["acc-1"]);
    assert_eq!(list[1].body_text, "@Ada Lovelace can you re-check?");
    assert_eq!(list[1].ticket, key("APP-1"));
    assert!(list[0].created_at < list[1].created_at);
}

#[test]
fn comments_bare_array_empty_and_broken() {
    let arr = r#"[{"id":7,"author":{"accountId":"a","displayName":"A"},"created":"2024-05-01T10:15:30.123+0000","body":"hi"}]"#;
    let (jira, _) = reader(
        Fake::default()
            .on(&comments_argv("APP-1"), ok(arr))
            .on(&comments_argv("APP-2"), ok(r#"{"comments":[]}"#))
            .on(&comments_argv("APP-3"), ok(r#"[{"author":{}}]"#))
            .on(&comments_argv("APP-4"), ok("not json")),
    );
    assert_eq!(jira.comments(&key("APP-1")).unwrap()[0].id, "7");
    assert!(jira.comments(&key("APP-2")).unwrap().is_empty());
    assert_eq!(
        jira.comments(&key("APP-3")).unwrap_err().kind(),
        ProviderErrorKind::Parse
    );
    assert_eq!(
        jira.comments(&key("APP-4")).unwrap_err().kind(),
        ProviderErrorKind::Parse
    );
}

#[test]
fn reads_never_issue_write_commands() {
    let (jira, fake) = reader(
        Fake::default()
            .on(&["--version"], ok("1.3.4"))
            .on(&["jira", "auth", "status"], ok("ok"))
            .on(&search_argv("q"), ok("[]"))
            .on(&comments_argv("APP-1"), ok("[]")),
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
    for call in fake.calls() {
        let joined = call.join(" ");
        assert!(
            !joined.contains("create") && !joined.contains("transition"),
            "{joined}"
        );
    }
    assert_eq!(probe(&Fake::default(), None).len(), 3);
}
