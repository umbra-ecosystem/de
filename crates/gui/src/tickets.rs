//! The synced tickets as the widgets model them.
//!
//! `cache.db` (the Jira mirror) joined with `state.db` (local tracking) becomes [`Ticket`]s. Jira's status
//! names are matched to the prototype's four through the configured names; any other status keeps Jira's own
//! spelling. The mapping is pure so it can be tested against temporary databases.

use std::collections::HashSet;
use std::sync::Mutex;

use de_core::config::Config;
use de_core::domain::LocalStatus;
use de_core::providers::TicketDetail;
use de_core::store::{Store, jira_cache, jira_comments, jira_details};
use de_widgets::sim::model::{
    Activation, Comment, Held, IssueLink, JiraStatus, Priority, Stage, Ticket,
};
use de_widgets::vm::{Block, Phase};

/// Every ticket the engine knows: the cached pool plus anything tracked by hand.
pub fn load(state: &Store, cache: &Store, config: &Config) -> eyre::Result<Vec<Ticket>> {
    let mut out = Vec::new();
    let details = jira_details::all(cache)?;
    for view in jira_cache::list_views(state, cache)? {
        let Some(jira) = view.jira else {
            // Tracked but never fetched: there is nothing to show yet except the key.
            if let Some(tracking) = view.tracking {
                let mut t = Ticket::blank(view.key.as_str(), view.key.as_str());
                t.stage = stage_of(tracking.status);
                out.push(t);
            }
            continue;
        };
        let mut t = Ticket::blank(jira.key.as_str(), &jira.title);
        let (status, name) = status_of(&jira.jira_status, config);
        t.jira = status;
        t.jira_name = name;
        t.priority = priority_of(jira.priority.as_deref());
        t.assignee = leak(jira.assignee.as_deref().unwrap_or(""));
        // Nothing is invented: what Jira has not told us yet stays empty and the views leave it out.
        t.kind = "";
        t.sprint = "";
        t.epic = "";
        t.fix_version = "";
        t.estimate = "";
        t.created = "";
        t.updated = leak(&format_time(jira.fetched_at));
        if let Some(detail) = details.get(&jira.key) {
            apply_detail(&mut t, detail, config.jira_account_id(), &|name| status_of(name, config).0);
        }
        t.comments = jira_comments::list_for_ticket(cache, &jira.key)?
            .into_iter()
            .enumerate()
            .map(|(i, c)| Comment {
                n: i as u32 + 1,
                who: c.author_name,
                at: format_time(c.created_at),
                body: blocks_of(
                    if c.rich.is_empty() { &c.body_text } else { &c.rich },
                    config.jira_account_id(),
                ),
            })
            .collect();
        if let Some(tracking) = view.tracking {
            t.stage = stage_of(tracking.status);
        }
        out.push(t);
    }
    Ok(out)
}

/// Fills the Jira-side fields of `t` from the cached detail.
fn apply_detail(
    t: &mut Ticket,
    d: &TicketDetail,
    me: Option<&str>,
    status_of_key: &dyn Fn(&str) -> JiraStatus,
) {
    t.kind = leak(d.issue_type.as_deref().unwrap_or(""));
    t.reporter = leak(d.reporter.as_deref().unwrap_or(""));
    t.sprint = leak(d.sprint.as_deref().unwrap_or(""));
    t.epic = leak(
        d.parent
            .as_ref()
            .map(|(key, title)| if title.is_empty() { key } else { title })
            .map_or("", String::as_str),
    );
    t.fix_version = leak(&d.fix_versions.join(", "));
    t.estimate = leak(d.original_estimate.as_deref().unwrap_or(""));
    if let Some(at) = d.created_at {
        t.created = leak(&format_time(at));
    }
    if let Some(at) = d.updated_at {
        t.updated = leak(&format_time(at));
        t.updated_at = Some(at);
    }
    t.labels = d.labels.iter().map(|l| leak(l)).collect();
    t.components = d.components.iter().map(|c| leak(c)).collect();
    t.desc = blocks_of(&d.description, me);
    t.attachments = d
        .attachments
        .iter()
        .map(|a| (leak(&a.name), leak(&size_text(a.bytes))))
        .collect();
    t.links = d
        .links
        .iter()
        .map(|l| IssueLink {
            rel: leak(&l.relation),
            key: l.key.as_str().into(),
            title: leak(&l.title),
            status: status_of_key(&l.status),
        })
        .collect();
    t.subtasks = d.subtasks.iter().map(|s| (s.done, leak(&s.title))).collect();
}

/// Jira rich text (the light markup of `de_core::providers::acli::adf::body_to_rich`) as blocks. `me` is the
/// reader's account id: a mention of them becomes `@[you]`, which the views highlight, and any other mention
/// keeps only the name.
pub fn blocks_of(rich: &str, me: Option<&str>) -> Vec<Block> {
    #[derive(PartialEq)]
    enum Open {
        None,
        Bullets,
        Numbers,
    }
    fn flush(out: &mut Vec<Block>, open: &mut Open, items: &mut Vec<String>) {
        let taken = std::mem::take(items);
        match open {
            Open::Bullets if !taken.is_empty() => out.push(Block::List(taken)),
            Open::Numbers if !taken.is_empty() => out.push(Block::Numbered(taken)),
            _ => {}
        }
        *open = Open::None;
    }

    let mut out = Vec::new();
    let mut open = Open::None;
    let mut items: Vec<String> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    for line in rich.lines().map(str::trim_end) {
        if let Some(lines) = &mut code {
            if line.trim() == "```" {
                out.push(Block::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let body = line.trim_start();
        if body.starts_with("```") {
            flush(&mut out, &mut open, &mut items);
            code = Some(Vec::new());
        } else if let Some(item) = body.strip_prefix("- ") {
            if open != Open::Bullets {
                flush(&mut out, &mut open, &mut items);
                open = Open::Bullets;
            }
            let item = match item {
                i if i.starts_with("[ ] ") => format!("\u{2610} {}", &i[4..]),
                i if i.starts_with("[x] ") => format!("\u{2611} {}", &i[4..]),
                i => i.to_string(),
            };
            // A nested item is flattened under its parent with a dash.
            let item = if indent > 0 { format!("\u{2013} {item}") } else { item };
            items.push(mentions(&item, me));
        } else if let Some(item) = numbered(body) {
            if open != Open::Numbers {
                flush(&mut out, &mut open, &mut items);
                open = Open::Numbers;
            }
            items.push(mentions(item, me));
        } else {
            flush(&mut out, &mut open, &mut items);
            if body.is_empty() || body == "---" {
                continue;
            }
            let hashes = body.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) && body[hashes..].starts_with(' ') {
                out.push(Block::Heading(mentions(body[hashes..].trim(), me)));
            } else if let Some(quoted) = body.strip_prefix('>') {
                out.push(Block::Quote(mentions(quoted.trim(), me)));
            } else if body.starts_with('|') {
                out.push(Block::Code(body.to_string()));
            } else {
                out.push(Block::Para(mentions(body, me)));
            }
        }
    }
    flush(&mut out, &mut open, &mut items);
    if let Some(lines) = code {
        out.push(Block::Code(lines.join("\n")));
    }
    out
}

/// `3. text` gives `text`.
fn numbered(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    (digits > 0).then(|| line[digits..].strip_prefix(". ")).flatten()
}

/// `@[Name|id]` becomes `@[you]` for the reader and `@[Name]` for anyone else.
fn mentions(text: &str, me: Option<&str>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("@[") {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        out.push_str(&rest[..start]);
        let inner = &rest[start + 2..start + end];
        match inner.split_once('|') {
            Some((_, id)) if me == Some(id) => out.push_str("@[you]"),
            Some((name, _)) => out.push_str(&format!("@[{name}]")),
            None => out.push_str(&format!("@[{inner}]")),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

fn size_text(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b / 1024),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// Jira's status name against the configured workflow names (case-insensitive).
pub fn status_of(name: &str, config: &Config) -> (JiraStatus, Option<String>) {
    let s = config.jira_statuses();
    let is = |other: &str| other.eq_ignore_ascii_case(name);
    let status = if is(s.review_name()) {
        JiraStatus::InReview
    } else if is(s.alpha_testing_name()) {
        JiraStatus::AlphaTesting
    } else if is(s.returned_name()) {
        JiraStatus::Returned
    } else if s.done_names().into_iter().any(is) {
        JiraStatus::Done
    } else {
        return (JiraStatus::Other, Some(name.to_string()));
    };
    (status, None)
}

pub fn priority_of(name: Option<&str>) -> Priority {
    match name.map(str::to_ascii_lowercase).as_deref() {
        Some("highest" | "blocker" | "critical") => Priority::Highest,
        Some("high" | "major") => Priority::High,
        Some("low" | "lowest" | "minor" | "trivial") => Priority::Low,
        _ => Priority::Medium,
    }
}

/// The widgets' stage for a local status. Records of an activation are not read yet, so an active ticket
/// carries an empty one.
pub fn stage_of(status: LocalStatus) -> Stage {
    let held = Held::default();
    let in_hand = |phase| Stage::InHand { phase, held: Held::default() };
    match status {
        LocalStatus::Claimed => in_hand(Phase::Claimed),
        LocalStatus::Reviewing => in_hand(Phase::Reviewing),
        LocalStatus::Parked => in_hand(Phase::Parked),
        LocalStatus::Active => Stage::Active {
            held,
            act: Activation {
                started_ms: 0,
                records: Vec::new(),
                overlay: None,
            },
            prep: None,
        },
        LocalStatus::Integrated => Stage::Integrated { held },
        LocalStatus::Done => Stage::Done { held },
    }
}

/// Text that lives as long as the app: the widgets' ticket fields are `&'static str`. Interned, so repeated
/// syncs do not grow memory.
pub fn leak(text: &str) -> &'static str {
    static SEEN: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut guard = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    if let Some(found) = set.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
    set.insert(leaked);
    leaked
}

/// `2026-10-01 09:40` in local time, from unix seconds.
pub fn format_time(secs: i64) -> String {
    crate::localtime::format(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use de_core::providers::RemoteTicket;
    use de_core::store::{Kind, tickets as tracking};

    fn remote(key: &str, status: &str, priority: &str) -> RemoteTicket {
        RemoteTicket {
            key: key.parse().unwrap(),
            title: format!("Title of {key}"),
            status: status.into(),
            priority: Some(priority.into()),
            assignee: Some("Alex Example".into()),
            url: None,
            updated_at: 1_700_000_000,
        }
    }

    #[test]
    fn a_ticket_with_detail_shows_what_jira_says_and_nothing_invented() {
        use de_core::providers::{Attachment, IssueLink as Link, Subtask};
        let state = Store::open_in_memory(Kind::State).unwrap();
        let cache = Store::open_in_memory(Kind::Cache).unwrap();
        jira_cache::upsert_remote(&cache, &remote("PROJ-1", "In Review", "High"), 10).unwrap();
        jira_cache::upsert_remote(&cache, &remote("PROJ-2", "In Review", "High"), 10).unwrap();
        let detail = TicketDetail {
            issue_type: Some("Bug".into()),
            reporter: Some("Riley Reporter".into()),
            created_at: Some(1_700_000_000),
            updated_at: Some(1_700_003_600),
            labels: vec!["web".into()],
            components: vec!["Frontend".into()],
            fix_versions: vec!["2.14.0".into(), "2.15.0".into()],
            sprint: Some("Sprint 41".into()),
            parent: Some(("PROJ-100".into(), "Reporting".into())),
            original_estimate: Some("3d".into()),
            description: "Intro\n- one\n- two\nOutro".into(),
            attachments: vec![Attachment { name: "spec.pdf".into(), bytes: 2048 }],
            links: vec![Link {
                relation: "blocks".into(),
                key: "PROJ-8".parse().unwrap(),
                title: "Downstream".into(),
                status: "Done".into(),
            }],
            subtasks: vec![Subtask { key: "PROJ-9".parse().unwrap(), title: "Tests".into(), done: true }],
        };
        jira_details::upsert(&cache, &"PROJ-1".parse().unwrap(), &detail, 10).unwrap();

        let mut all = load(&state, &cache, &Config::default()).unwrap();
        all.sort_by(|a, b| a.key.cmp(&b.key));
        let t = &all[0];
        assert_eq!(t.kind, "Bug");
        assert_eq!(t.reporter, "Riley Reporter");
        assert_eq!((t.sprint, t.epic, t.fix_version, t.estimate), ("Sprint 41", "Reporting", "2.14.0, 2.15.0", "3d"));
        assert_eq!(t.updated_at, Some(1_700_003_600), "kept as a time, for the age");
        assert_eq!(t.created, format_time(1_700_000_000));
        assert_eq!(t.updated, format_time(1_700_003_600));
        assert_eq!(t.labels, ["web"]);
        assert_eq!(t.components, ["Frontend"]);
        assert_eq!(
            t.desc,
            [
                Block::Para("Intro".into()),
                Block::List(vec!["one".into(), "two".into()]),
                Block::Para("Outro".into()),
            ]
        );
        assert_eq!(t.attachments, [("spec.pdf", "2 KB")]);
        assert_eq!(t.links[0].status, JiraStatus::Done);
        assert_eq!(t.subtasks, [(true, "Tests")]);

        // A ticket whose detail has not been fetched shows none of the prototype's made-up values.
        let bare = &all[1];
        assert_eq!((bare.kind, bare.sprint, bare.fix_version, bare.estimate, bare.created), ("", "", "", "", ""));
        assert!(bare.desc.is_empty() && bare.links.is_empty());
        assert_eq!(bare.updated_at, None);
    }

    #[test]
    fn rich_text_becomes_blocks_with_lines_lists_code_and_mentions() {
        let rich = "## Plan\n\nFirst line\nsecond line\n\n- one\n  - nested\n- [x] done\n- [ ] todo\n\n3. third\n4. fourth\n\n```\nlet x = 1;\nlet y = 2;\n```\n\n> quoted\n\nAsk @[Ada|a1] and @[Me|acc-me]\n\n---\n\n| a | b |";
        assert_eq!(
            blocks_of(rich, Some("acc-me")),
            [
                Block::Heading("Plan".into()),
                Block::Para("First line".into()),
                Block::Para("second line".into()),
                Block::List(vec![
                    "one".into(),
                    "\u{2013} nested".into(),
                    "\u{2611} done".into(),
                    "\u{2610} todo".into(),
                ]),
                Block::Numbered(vec!["third".into(), "fourth".into()]),
                Block::Code("let x = 1;\nlet y = 2;".into()),
                Block::Quote("quoted".into()),
                Block::Para("Ask @[Ada] and @[you]".into()),
                Block::Code("| a | b |".into()),
            ]
        );
    }

    #[test]
    fn an_unterminated_code_fence_and_plain_old_comments_still_render() {
        assert_eq!(
            blocks_of("```\nopen", None),
            [Block::Code("open".into())]
        );
        // A comment cached before rich text existed is flat text with newlines: one paragraph per line.
        assert_eq!(
            blocks_of("Looks good\nship it", None),
            [Block::Para("Looks good".into()), Block::Para("ship it".into())]
        );
        assert!(blocks_of("", None).is_empty());
    }

    #[test]
    fn comments_show_their_rich_body_and_fall_back_to_the_flat_text() {
        use de_core::providers::RemoteComment;
        let state = Store::open_in_memory(Kind::State).unwrap();
        let cache = Store::open_in_memory(Kind::Cache).unwrap();
        let key: de_core::domain::TicketKey = "PROJ-1".parse().unwrap();
        jira_cache::upsert_remote(&cache, &remote("PROJ-1", "In Review", "High"), 10).unwrap();
        let c = |id: &str, flat: &str, rich: &str| RemoteComment {
            id: id.into(),
            ticket: key.clone(),
            author_account_id: "a".into(),
            author_name: "Ann".into(),
            body_text: flat.into(),
            rich: rich.into(),
            mentions: vec![],
            created_at: 100 + i64::from(id == "2"),
        };
        jira_comments::replace_for_ticket(
            &cache,
            &key,
            &[c("1", "Plan\nstep", "## Plan\n\n- step"), c("2", "old one\nline two", "")],
            10,
        )
        .unwrap();

        let all = load(&state, &cache, &Config::default()).unwrap();
        assert_eq!(
            all[0].comments[0].body,
            [Block::Heading("Plan".into()), Block::List(vec!["step".into()])]
        );
        assert_eq!(
            all[0].comments[1].body,
            [Block::Para("old one".into()), Block::Para("line two".into())]
        );
    }

    #[test]
    fn formats_unix_time() {
        // In local time, so the expected text is built the same way; the arithmetic is tested in `localtime`.
        assert_eq!(format_time(0), crate::localtime::format(0));
        assert_eq!(format_time(1_700_000_000), crate::localtime::format(1_700_000_000));
    }

    #[test]
    fn statuses_use_configured_names_and_keep_unknown_ones() {
        let config = Config::default();
        assert_eq!(status_of("in review", &config), (JiraStatus::InReview, None));
        assert_eq!(status_of("Returned", &config).0, JiraStatus::Returned);
        assert_eq!(status_of("Done", &config).0, JiraStatus::Done);
        assert_eq!(
            status_of("Blocked", &config),
            (JiraStatus::Other, Some("Blocked".into()))
        );
    }

    #[test]
    fn joins_the_mirror_with_local_tracking() {
        let state = Store::open_in_memory(Kind::State).unwrap();
        let cache = Store::open_in_memory(Kind::Cache).unwrap();
        jira_cache::upsert_remote(&cache, &remote("PROJ-1", "In Review", "High"), 10).unwrap();
        jira_cache::upsert_remote(&cache, &remote("PROJ-2", "Returned", "Low"), 10).unwrap();
        tracking::claim(&state, &"PROJ-2".parse().unwrap(), 20).unwrap();

        let mut all = load(&state, &cache, &Config::default()).unwrap();
        all.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].title, "Title of PROJ-1");
        assert_eq!(all[0].priority, Priority::High);
        assert_eq!(all[0].assignee, "Alex Example");
        assert!(all[0].stage.local().is_none(), "unclaimed stays unclaimed");
        assert_eq!(all[1].jira, JiraStatus::Returned);
        assert_eq!(all[1].stage.name(), "claimed");
    }
}
