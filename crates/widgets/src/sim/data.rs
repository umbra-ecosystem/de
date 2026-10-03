//! Invented tickets, branches and diffs. Nothing here is real; it mirrors the prototype's `data.js`.

use std::collections::BTreeMap;

use super::model::*;
use crate::vm::{Block, Branch, LineAnchor, Phase, PrNumber, RepoName};

fn p(x: &str) -> Block {
    Block::Para(x.to_string())
}
fn h(x: &str) -> Block {
    Block::Heading(x.to_string())
}
fn ul(items: &[&str]) -> Block {
    Block::List(items.iter().map(|s| s.to_string()).collect())
}
fn code(x: &str) -> Block {
    Block::Code(x.to_string())
}

fn ctx(o: u32, n: u32, x: &str) -> Line {
    Line {
        k: Kind::Ctx,
        o: Some(o),
        n: Some(n),
        x: x.to_string(),
    }
}
fn add(n: u32, x: &str) -> Line {
    Line {
        k: Kind::Add,
        o: None,
        n: Some(n),
        x: x.to_string(),
    }
}
fn del(o: u32, x: &str) -> Line {
    Line {
        k: Kind::Del,
        o: Some(o),
        n: None,
        x: x.to_string(),
    }
}

fn file(path: &str, adds: u32, dels: u32, lines: Vec<Line>) -> FileDiff {
    FileDiff {
        path: path.to_string(),
        adds,
        dels,
        lines,
    }
}

fn comment(n: u32, who: &str, at: &str, body: Vec<Block>) -> Comment {
    Comment {
        n,
        who: who.to_string(),
        at: at.to_string(),
        body,
    }
}

fn reviewers(first: &str) -> Vec<Reviewer> {
    vec![
        Reviewer {
            name: first.to_string(),
            approved: false,
        },
        Reviewer {
            name: ME.to_string(),
            approved: false,
        },
    ]
}

fn reviewers_approved(first: &str) -> Vec<Reviewer> {
    vec![
        Reviewer {
            name: first.to_string(),
            approved: true,
        },
        Reviewer {
            name: ME.to_string(),
            approved: false,
        },
    ]
}

fn pr(
    repo: &str,
    id: u32,
    title: &str,
    src: &str,
    dst: &str,
    rev: Vec<Reviewer>,
    files: Vec<FileDiff>,
) -> Pr {
    Pr {
        repo: repo.into(),
        id: PrNumber(id),
        title: title.to_string(),
        src: src.into(),
        dst: dst.into(),
        url: None,
        updated_seq: 1,
        reviewers: rev,
        files,
        threads: Vec::new(),
        since: None,
    }
}

fn thread(
    id: u64,
    file: &str,
    line: Option<LineAnchor>,
    author: &str,
    text: &str,
    resolved: bool,
) -> Thread {
    Thread {
        id: ThreadId(id),
        file: file.to_string(),
        line,
        author: author.to_string(),
        text: text.to_string(),
        resolved,
    }
}

fn cands(list: &[(&str, &[&str])]) -> BTreeMap<RepoName, Vec<Branch>> {
    list.iter()
        .map(|(r, b)| ((*r).into(), b.iter().map(|s| Branch::from(*s)).collect()))
        .collect()
}

fn checklist(items: &[&str]) -> Vec<(String, bool)> {
    items.iter().map(|s| (s.to_string(), false)).collect()
}

fn landing(repo: &str, commit: &str, at: &str, run: u32) -> Landing {
    Landing {
        repo: repo.into(),
        commit: commit.to_string(),
        at: at.to_string(),
        deploy: deployed(run),
    }
}

fn integrated(landings: Vec<Landing>, draft: Draft) -> Stage {
    Stage::Integrated {
        held: Held {
            review: Some(ReviewMark(1)),
            shipped: Shipped {
                landings,
                drafts: vec![draft],
            },
        },
    }
}

fn deployed(run: u32) -> Deploy {
    Deploy {
        state: DeployState::Deployed,
        run,
        since: -1_000_000_000,
        step: "Deploy alpha".to_string(),
        log: None,
        uat_moved: None,
        url: None,
    }
}

pub fn seed_tickets() -> Vec<Ticket> {
    let mut out = Vec::new();

    let mut t = Ticket::blank("PROJ-142", "Fix login redirect after session expiry");
    t.kind = "Bug";
    t.priority = Priority::Medium;
    t.assignee = "Sam Ortiz";
    t.reporter = "Jane Doe";
    t.epic = "Auth hardening";
    t.estimate = "3 pts";
    t.created = "3 days ago";
    t.updated = "1 hour ago";
    t.labels = vec!["auth", "regression"];
    t.components = vec!["api-client", "web"];
    t.desc = vec![
        p(
            "Users whose session expires while they are on a deep link are sent to /home after logging back in, instead of the page they were on. Support has seen it on three accounts this week.",
        ),
        h("Steps to reproduce"),
        ul(&[
            "Log in and open /orders/4821",
            "Wait for the session to expire (15 min) or clear the session cookie",
            "Click any action and log in again",
        ]),
        p("Expected: back on /orders/4821. Actual: /home."),
        code("GET /login?next=%2Forders%2F4821   ->   302 /home"),
    ];
    t.ac = vec![
        "The deep link is preserved through login".into(),
        "Only same-origin next values are accepted".into(),
        "Existing login form tests still pass".into(),
    ];
    t.comments = vec![
        comment(
            1,
            "Jane Doe",
            "yesterday 16:40",
            vec![p(
                "Three customers hit this today. One lost an unsaved order form, so this is more than cosmetic.",
            )],
        ),
        comment(
            2,
            "Sam Ortiz",
            "yesterday 17:05",
            vec![p(
                "Root cause is in api-client: landing() ignores next. The fix is on two branches; the web side only passes the param through. @[you] can you take the review?",
            )],
        ),
        comment(
            3,
            "Jane Doe",
            "today 09:12",
            vec![p(
                "Please also check the open-redirect case. Security asked about it.",
            )],
        ),
    ];
    t.attachments = vec![
        ("session-expiry.mov", "2.1 MB"),
        ("network-trace.har", "340 KB"),
    ];
    t.links = vec![
        IssueLink {
            rel: "relates to",
            key: "PROJ-127".into(),
            title: "Session cookie SameSite",
            status: JiraStatus::AlphaTesting,
        },
        IssueLink {
            rel: "is blocked by",
            key: "PROJ-098".into(),
            title: "Refactor login module",
            status: JiraStatus::Done,
        },
    ];
    t.subtasks = vec![(true, "Add regression test"), (false, "Update the runbook")];
    t.cands = cands(&[
        ("api-client", &["feature/PROJ-142-redirect"]),
        ("web", &["feature/PROJ-142-web"]),
    ]);
    t.stage = Stage::InHand {
        phase: Phase::Claimed,
        held: Held::default(),
    };
    t.seen_n = 1;
    t.checklist = checklist(&[
        "Deep link survives login",
        "External next value is rejected",
        "Logout clears the session",
    ]);
    let mut api = pr(
        "api-client",
        212,
        "PROJ-142 Preserve next through login",
        "feature/PROJ-142-redirect",
        "develop",
        reviewers("Jane Doe"),
        vec![
            file(
                "src/redirect.rs",
                9,
                1,
                vec![
                    ctx(38, 38, "use crate::user::User;"),
                    ctx(39, 39, ""),
                    ctx(
                        40,
                        40,
                        "pub fn target(user: &User, next: Option<&str>) -> Url {",
                    ),
                    del(41, "    Url::parse(\"/home\").unwrap()"),
                    add(41, "    match next.filter(|n| is_same_origin(n)) {"),
                    add(
                        42,
                        "        Some(n) => Url::parse(n).unwrap_or_else(|_| user.landing()),",
                    ),
                    add(43, "        None => user.landing(),"),
                    add(44, "    }"),
                    ctx(42, 45, "}"),
                    ctx(43, 46, ""),
                    add(47, "fn is_same_origin(n: &str) -> bool {"),
                    add(48, "    n.starts_with('/') && !n.starts_with(\"//\")"),
                    add(49, "}"),
                ],
            ),
            file(
                "tests/redirect_test.rs",
                14,
                0,
                vec![
                    ctx(1, 1, "use api_client::redirect::target;"),
                    ctx(2, 2, ""),
                    add(3, "#[test]"),
                    add(4, "fn keeps_a_same_origin_next() {"),
                    add(5, "    let url = target(&user(), Some(\"/orders/4821\"));"),
                    add(6, "    assert_eq!(url.path(), \"/orders/4821\");"),
                    add(7, "}"),
                    add(8, "#[test]"),
                    add(9, "fn rejects_an_external_next() {"),
                    add(
                        10,
                        "    let url = target(&user(), Some(\"//evil.example/x\"));",
                    ),
                    add(11, "    assert_eq!(url.path(), user().landing().path());"),
                    add(12, "}"),
                ],
            ),
        ],
    );
    api.threads.push(thread(
        1,
        "src/redirect.rs",
        Some(LineAnchor::New(42)),
        "Jane Doe",
        "Should we log when we reject a next value? Security asked for that.",
        true,
    ));
    let web = pr(
        "web",
        488,
        "PROJ-142 Pass next to the login call",
        "feature/PROJ-142-web",
        "develop",
        reviewers("Jane Doe"),
        vec![file(
            "src/session.rs",
            4,
            2,
            vec![
                ctx(20, 20, "pub fn login(req: &Request) -> Result<Session> {"),
                del(21, "    let url = api::login(&req.credentials)?;"),
                add(21, "    let next = req.query(\"next\");"),
                add(
                    22,
                    "    let url = api::login(&req.credentials, next.as_deref())?;",
                ),
                ctx(22, 23, "    Ok(Session::from(url))"),
                ctx(23, 24, "}"),
            ],
        )],
    );
    t.prs = vec![api, web];
    out.push(t);

    let mut t = Ticket::blank("PROJ-150", "Fix header overflow on narrow screens");
    t.kind = "Story";
    t.priority = Priority::High;
    t.assignee = "Priya Nair";
    t.reporter = "Marta Lind";
    t.epic = "Mobile polish";
    t.estimate = "2 pts";
    t.created = "2 days ago";
    t.updated = "3 hours ago";
    t.labels = vec!["ui", "mobile"];
    t.components = vec!["web"];
    t.desc = vec![
        p(
            "On viewports narrower than 380px the header menu wraps under the logo and pushes the page sideways. The search field is the culprit.",
        ),
        p("Taken on an iPhone SE: ![iphone-se.png]"),
        h("Acceptance"),
        ul(&[
            "No horizontal scroll at 320px",
            "Menu stays reachable",
            "Desktop layout unchanged",
        ]),
    ];
    t.ac = vec![
        "No horizontal scroll at 320px".into(),
        "Menu stays reachable".into(),
        "Desktop layout unchanged".into(),
    ];
    t.comments = vec![
        comment(
            1,
            "Marta Lind",
            "2 days ago",
            vec![p(
                "Screenshot attached from an iPhone SE. It looks worse in landscape.",
            )],
        ),
        comment(
            2,
            "Priya Nair",
            "yesterday 11:20",
            vec![p(
                "Fixed with a flex-wrap and a shrinking search. Two branches exist because I restarted; the -old one can be ignored.",
            )],
        ),
    ];
    t.attachments = vec![("iphone-se.png", "480 KB")];
    t.subtasks = vec![(true, "Check landscape")];
    t.cands = cands(&[(
        "web",
        &["feature/PROJ-150-header", "feature/PROJ-150-header-old"],
    )]);
    t.checklist = checklist(&["No horizontal scroll at 320px", "Menu reachable"]);
    let mut w = pr(
        "web",
        490,
        "PROJ-150 Wrap the header on narrow screens",
        "feature/PROJ-150-header",
        "develop",
        reviewers("Marta Lind"),
        vec![
            file(
                "src/header.css",
                6,
                2,
                vec![
                    ctx(10, 10, ".header {"),
                    del(11, "  display: flex;"),
                    add(11, "  display: flex;"),
                    add(12, "  flex-wrap: wrap;"),
                    add(13, "  gap: 8px;"),
                    ctx(12, 14, "}"),
                    ctx(13, 15, ""),
                    ctx(14, 16, ".header .search {"),
                    del(15, "  width: 320px;"),
                    add(17, "  flex: 1 1 120px;"),
                    add(18, "  min-width: 0;"),
                    ctx(16, 19, "}"),
                ],
            ),
            file(
                "src/Header.tsx",
                2,
                1,
                vec![
                    ctx(8, 8, "export function Header() {"),
                    del(9, "  return <nav className=\"header\">"),
                    add(9, "  return <nav className=\"header\" aria-label=\"Main\">"),
                    ctx(10, 10, "    <Logo />"),
                ],
            ),
        ],
    );
    w.threads.push(thread(
        2,
        "src/header.css",
        Some(LineAnchor::New(12)),
        "Marta Lind",
        "flex-wrap breaks the logo alignment on tablets. Can we keep the logo on its own row?",
        false,
    ));
    t.prs = vec![w];
    out.push(t);

    let mut t = Ticket::blank("PROJ-139", "Payment retry double-charges on timeout");
    t.kind = "Bug";
    t.priority = Priority::Highest;
    t.assignee = "Sam Ortiz";
    t.reporter = "Omar Haddad";
    t.sprint = "Hotfix";
    t.epic = "Payments";
    t.fix_version = "2.13.1";
    t.estimate = "2 pts";
    t.created = "today 07:50";
    t.updated = "30 min ago";
    t.labels = vec!["payments", "hotfix"];
    t.components = vec!["api-client", "web"];
    t.desc = vec![
        p(
            "When the payment provider times out, the client retries with a new idempotency key. If the first request actually succeeded the customer is charged twice.",
        ),
        h("Impact"),
        ul(&[
            "11 customers charged twice since Monday",
            "Refunds are being handled manually",
        ]),
        p(
            "Fix: reuse the idempotency key across retries. Targets the production branch because this is live.",
        ),
    ];
    t.ac = vec![
        "A retry reuses the original idempotency key".into(),
        "A timed-out request is never re-sent with a new key".into(),
    ];
    t.comments = vec![
        comment(
            1,
            "Omar Haddad",
            "today 07:55",
            vec![p("This is costing us real money. Fastest safe fix please.")],
        ),
        comment(
            2,
            "Sam Ortiz",
            "today 09:30",
            vec![p(
                "Both PRs target master and main. Needs @[you] to review and test.",
            )],
        ),
    ];
    t.attachments = vec![("double-charge-report.csv", "18 KB")];
    t.links = vec![IssueLink {
        rel: "is caused by",
        key: "PROJ-071".into(),
        title: "Client retry policy",
        status: JiraStatus::Done,
    }];
    t.cands = cands(&[
        ("api-client", &["hotfix/PROJ-139-idempotency"]),
        ("web", &["hotfix/PROJ-139-web"]),
    ]);
    t.checklist = checklist(&["Retry keeps the same key", "No second charge on timeout"]);
    t.prs = vec![
        pr(
            "api-client",
            214,
            "PROJ-139 Reuse idempotency key on retry",
            "hotfix/PROJ-139-idempotency",
            "master",
            reviewers("Omar Haddad"),
            vec![file(
                "src/payments.rs",
                5,
                3,
                vec![
                    ctx(
                        60,
                        60,
                        "fn charge(&self, req: &Charge) -> Result<Receipt> {",
                    ),
                    del(61, "    let key = Uuid::new_v4();"),
                    add(
                        61,
                        "    let key = req.idempotency_key.get_or_insert_with(Uuid::new_v4);",
                    ),
                    ctx(62, 62, "    self.post(\"/charges\", req, key)"),
                    ctx(63, 63, "}"),
                ],
            )],
        ),
        pr(
            "web",
            491,
            "PROJ-139 Send a stable key from the checkout",
            "hotfix/PROJ-139-web",
            "main",
            reviewers("Omar Haddad"),
            vec![file(
                "src/checkout.ts",
                3,
                1,
                vec![
                    ctx(14, 14, "async function pay(order: Order) {"),
                    del(15, "  return api.charge(order);"),
                    add(15, "  order.key ??= crypto.randomUUID();"),
                    add(16, "  return api.charge(order);"),
                    ctx(16, 17, "}"),
                ],
            )],
        ),
    ];
    out.push(t);

    let mut t = Ticket::blank("PROJ-131", "Staging redirect loop");
    t.kind = "Task";
    t.priority = Priority::Low;
    t.jira = JiraStatus::Returned;
    t.assignee = "Sam Ortiz";
    t.reporter = "Jane Doe";
    t.sprint = "Sprint 40";
    t.epic = "Auth hardening";
    t.fix_version = "2.13.0";
    t.created = "9 days ago";
    t.updated = "2 hours ago";
    t.labels = vec!["auth"];
    t.components = vec!["web"];
    t.desc = vec![
        p("Expired sessions loop between /login and /home on staging only."),
        ul(&[
            "Reproduce with a stale cookie",
            "Compare staging and production cookie domains",
        ]),
    ];
    t.ac = vec!["No redirect loop with an expired cookie".into()];
    t.comments = vec![
        comment(
            1,
            "Sam Ortiz",
            "6 days ago",
            vec![p("Deployed to alpha, cookie domain fixed.")],
        ),
        comment(
            2,
            "Jane Doe",
            "today 10:05",
            vec![p(
                "Sent back. @[you] can you re-check the redirect on staging? It still loops for expired sessions.",
            )],
        ),
    ];
    t.links = vec![IssueLink {
        rel: "relates to",
        key: "PROJ-142".into(),
        title: "Fix login redirect after session expiry",
        status: JiraStatus::InReview,
    }];
    t.cands = cands(&[("web", &["feature/PROJ-131-cookie"])]);
    t.seen_n = 1;
    t.stage = integrated(
        vec![landing("web", "2b90cd3", "6 days ago", 401)],
        Draft {
            id: "d1".into(),
            body: "Deployed to alpha.".into(),
            posted: true,
        },
    );
    t.prs = vec![pr(
        "web",
        470,
        "PROJ-131 Scope the session cookie",
        "feature/PROJ-131-cookie",
        "develop",
        reviewers("Jane Doe"),
        vec![file(
            "src/cookie.ts",
            2,
            1,
            vec![
                ctx(3, 3, "export const cookie = {"),
                del(4, "  domain: \".acme.dev\","),
                add(4, "  domain: \"staging.acme.dev\","),
                ctx(5, 5, "};"),
            ],
        )],
    )];
    out.push(t);

    let mut t = Ticket::blank("PROJ-127", "Session cookie SameSite");
    t.kind = "Story";
    t.jira = JiraStatus::AlphaTesting;
    t.assignee = "Priya Nair";
    t.reporter = "Omar Haddad";
    t.sprint = "Sprint 40";
    t.epic = "Auth hardening";
    t.fix_version = "2.13.0";
    t.estimate = "2 pts";
    t.created = "12 days ago";
    t.updated = "yesterday";
    t.labels = vec!["auth", "security"];
    t.components = vec!["api-client", "web"];
    t.desc = vec![
        p(
            "Set SameSite=Lax on the session cookie and document the cross-site flows that need None.",
        ),
        ul(&[
            "api-client sets the attribute",
            "web forwards it on redirects",
        ]),
    ];
    t.ac = vec![
        "Cookie carries SameSite=Lax".into(),
        "Docs list the exceptions".into(),
    ];
    t.comments = vec![comment(
        1,
        "Priya Nair",
        "yesterday 15:10",
        vec![
            p("Deployed to alpha."),
            p("api-client: run #311 https://github.com/acme/api-client/actions/runs/311"),
            p("web: run #398 https://github.com/acme/web/actions/runs/398"),
        ],
    )];
    t.links = vec![IssueLink {
        rel: "relates to",
        key: "PROJ-142".into(),
        title: "Fix login redirect after session expiry",
        status: JiraStatus::InReview,
    }];
    t.cands = cands(&[
        ("api-client", &["feature/PROJ-127-samesite"]),
        ("web", &["feature/PROJ-127-samesite"]),
    ]);
    t.seen_n = 1;
    t.stage = integrated(
        vec![
            landing("api-client", "8c01d22", "yesterday", 311),
            landing("web", "44ab0f9", "yesterday", 398),
        ],
        Draft {
            id: "d0".into(),
            body: "Deployed to alpha.".into(),
            posted: true,
        },
    );
    t.prs = vec![
        pr(
            "api-client",
            205,
            "PROJ-127 SameSite=Lax",
            "feature/PROJ-127-samesite",
            "develop",
            reviewers_approved("Omar Haddad"),
            vec![file(
                "src/cookie.rs",
                2,
                0,
                vec![add(12, "    .same_site(SameSite::Lax)")],
            )],
        ),
        pr(
            "web",
            469,
            "PROJ-127 Forward SameSite",
            "feature/PROJ-127-samesite",
            "develop",
            reviewers_approved("Omar Haddad"),
            vec![file(
                "src/session.rs",
                3,
                1,
                vec![
                    ctx(30, 30, "fn store(c: Cookie) {"),
                    add(31, "    let c = c.same_site(Lax);"),
                ],
            )],
        ),
    ];
    out.push(t);

    let mut t = Ticket::blank("PROJ-160", "Update onboarding copy");
    t.priority = Priority::Low;
    t.assignee = "Marta Lind";
    t.reporter = "Marta Lind";
    t.epic = "Docs";
    t.created = "yesterday";
    t.updated = "yesterday";
    t.labels = vec!["docs"];
    t.components = vec!["docs"];
    t.desc = vec![p(
        "Replace the old welcome text with the new wording from marketing.",
    )];
    t.ac = vec!["New wording in the getting started page".into()];
    t.cands = cands(&[("docs", &["docs/PROJ-160-onboarding"])]);
    t.checklist = checklist(&["Page reads correctly"]);
    t.prs = vec![pr(
        "docs",
        58,
        "PROJ-160 Onboarding copy",
        "docs/PROJ-160-onboarding",
        "master",
        reviewers("Marta Lind"),
        vec![file(
            "getting-started.md",
            4,
            3,
            vec![
                ctx(1, 1, "# Getting started"),
                del(2, "Welcome aboard."),
                add(
                    2,
                    "Welcome to Acme. Here is how to get going in ten minutes.",
                ),
            ],
        )],
    )];
    out.push(t);

    let mut t = Ticket::blank("PROJ-163", "Export button missing on the reports page");
    t.kind = "Story";
    t.assignee = "Priya Nair";
    t.reporter = "Jane Doe";
    t.epic = "Reporting";
    t.estimate = "2 pts";
    t.created = "today 08:50";
    t.updated = "today 09:20";
    t.labels = vec!["reports"];
    t.components = vec!["web"];
    t.desc = vec![p(
        "The CSV export button disappeared from the reports page after the toolbar refactor. It should sit next to the date filter again.",
    )];
    t.ac = vec![
        "Export button visible on /reports".into(),
        "Export respects the date filter".into(),
    ];
    t.comments = vec![comment(
        1,
        "Priya Nair",
        "today 09:20",
        vec![p("Branch is pushed, I will open the PR after lunch.")],
    )];
    t.cands = cands(&[("web", &["feature/PROJ-163-export"])]);
    t.checklist = checklist(&[
        "Button visible next to the date filter",
        "Export respects the filter",
    ]);
    out.push(t);

    out
}

/// A ticket that appears on a later sync.
pub fn arriving_ticket() -> Ticket {
    let mut t = Ticket::blank("PROJ-155", "Toast overlaps the footer on iPad");
    t.kind = "Bug";
    t.priority = Priority::High;
    t.assignee = "Priya Nair";
    t.reporter = "Marta Lind";
    t.epic = "Mobile polish";
    t.created = "today 10:40";
    t.labels = vec!["ui"];
    t.components = vec!["web"];
    t.desc = vec![p(
        "On iPad in portrait the success toast covers the footer links until it is dismissed.",
    )];
    t.ac = vec!["Toast never covers the footer".into()];
    t.comments = vec![comment(
        1,
        "Marta Lind",
        "today 10:41",
        vec![p(
            "Seen on iPad Air. @[you] you had this pattern in the header ticket, thoughts?",
        )],
    )];
    t.cands = cands(&[("web", &["feature/PROJ-155-toast"])]);
    t.checklist = checklist(&["Toast clear of the footer"]);
    t.prs = vec![pr(
        "web",
        495,
        "PROJ-155 Lift the toast above the footer",
        "feature/PROJ-155-toast",
        "develop",
        reviewers("Marta Lind"),
        vec![file(
            "src/toast.css",
            2,
            1,
            vec![
                ctx(5, 5, ".toast {"),
                del(6, "  bottom: 0;"),
                add(6, "  bottom: 64px;"),
                ctx(7, 7, "}"),
            ],
        )],
    )];
    t
}

/// What a freshly opened PR looks like (sync arrivals and the simulate panel).
pub fn open_fake_prs(t: &mut Ticket, repos: &[RepoCfg], serial: u32) {
    let cands = t.cands.clone();
    for (repo, branches) in cands {
        if t.prs.iter().any(|x| x.repo == repo) {
            continue;
        }
        let src = t
            .link
            .get(&repo)
            .cloned()
            .unwrap_or_else(|| branches[0].clone());
        let base = repos
            .iter()
            .find(|r| r.name == repo)
            .map_or("develop", |r| r.base.as_str());
        let id = 500 + t.prs.len() as u32 + serial % 40;
        t.prs.push(pr(
            &repo,
            id,
            &format!("{} {}", t.key, t.title),
            &src,
            base,
            reviewers(t.reporter),
            vec![file(
                "src/reports.rs",
                3,
                1,
                vec![
                    ctx(12, 12, "fn toolbar() -> Toolbar {"),
                    del(13, "    Toolbar::new()"),
                    add(13, "    Toolbar::new()"),
                    add(14, "        .with_filter(date_filter())"),
                    add(15, "        .with_export(csv_export())"),
                    ctx(14, 16, "}"),
                ],
            )],
        ));
    }
}
