//! Atlassian Document Format (ADF) to plain text, and @mention extraction.
//!
//! ADF is the JSON tree Jira Cloud uses for comment bodies:
//! <https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/>.
//! A `mention` node carries the mentioned user's account id in `attrs.id`. Everything here
//! is tolerant: unknown node types are rendered from their children (or their `text`), never
//! rejected, because Atlassian adds node types over time.
//!
//! Comment bodies are not always ADF, so [`body_to_text`] also handles plain text and Jira
//! wiki markup (`[~accountid:ID]` mentions) and HTML with `data-account-id` attributes.

use serde_json::Value;

/// A comment body flattened to text plus the account ids it mentions (deduplicated, in
/// order of appearance).
pub fn body_to_text(body: &Value) -> (String, Vec<String>) {
    match body {
        Value::Null => (String::new(), Vec::new()),
        Value::String(s) => {
            let trimmed = s.trim_start();
            if trimmed.starts_with('{')
                && let Ok(doc) = serde_json::from_str::<Value>(trimmed)
                && doc.get("type").is_some()
            {
                return adf_body(&doc);
            }
            string_body(s)
        }
        Value::Object(_) => adf_body(body),
        other => (other.to_string(), Vec::new()),
    }
}

fn adf_body(doc: &Value) -> (String, Vec<String>) {
    let mut mentions = Vec::new();
    collect_mentions(doc, &mut mentions);
    (adf_to_text(doc), mentions)
}

/// Renders an ADF document (or any node) as plain text.
pub fn adf_to_text(node: &Value) -> String {
    let text = render_block(node);
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    lines.join("\n").trim().to_string()
}

/// Every `mention` node's `attrs.id` under `node`, deduplicated, in document order.
pub fn collect_mentions(node: &Value, out: &mut Vec<String>) {
    if node.get("type").and_then(Value::as_str) == Some("mention")
        && let Some(id) = node.pointer("/attrs/id").and_then(Value::as_str)
        && !id.is_empty()
        && !out.iter().any(|m| m == id)
    {
        out.push(id.into());
    }
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        for c in children {
            collect_mentions(c, out);
        }
    }
}

// ---- plain text / wiki markup / HTML -------------------------------------------------

fn string_body(s: &str) -> (String, Vec<String>) {
    let mut mentions = Vec::new();
    let mut text = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("[~accountid:") {
        text.push_str(&rest[..start]);
        let after = &rest[start + "[~accountid:".len()..];
        match after.find(']') {
            Some(end) if end > 0 && !after[..end].contains(char::is_whitespace) => {
                let id = &after[..end];
                push_unique(&mut mentions, id);
                text.push('@');
                text.push_str(id);
                rest = &after[end + 1..];
            }
            _ => {
                text.push_str("[~accountid:");
                rest = after;
            }
        }
    }
    text.push_str(rest);
    // Rendered HTML: `<a ... data-account-id="ID" ...>`.
    let mut rest = s;
    while let Some(pos) = rest.find("data-account-id=") {
        let after = &rest[pos + "data-account-id=".len()..];
        if let Some(q @ ('"' | '\'')) = after.chars().next()
            && let Some(end) = after[1..].find(q)
        {
            let id = &after[1..1 + end];
            if !id.is_empty() {
                push_unique(&mut mentions, id);
            }
        }
        rest = after;
    }
    (text, mentions)
}

fn push_unique(list: &mut Vec<String>, id: &str) {
    if !list.iter().any(|m| m == id) {
        list.push(id.into());
    }
}

// ---- ADF rendering --------------------------------------------------------------------

fn node_type(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or("")
}

fn children(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn attr_str<'a>(node: &'a Value, name: &str) -> Option<&'a str> {
    node.get("attrs")
        .and_then(|a| a.get(name))
        .and_then(Value::as_str)
}

fn is_inline(t: &str) -> bool {
    matches!(
        t,
        "text"
            | "hardBreak"
            | "mention"
            | "emoji"
            | "inlineCard"
            | "status"
            | "date"
            | "mediaInline"
            | "placeholder"
    )
}

/// What a media node (a file dropped into the body) reads as: a reference naming the file when
/// the node says which one it is, else the bare placeholder. Jira names the file in
/// `attrs.alt` when it was inserted with its name; the `id` is a media-store id rather than the
/// attachment, so there is nothing to match it against. The reference reads `![name]`, which the
/// views render as a chip where the file sits.
fn media_ref(node: &Value) -> String {
    // A group holds several files, each named (or not) on its own.
    if node_type(node) == "mediaGroup" {
        let refs = children(node)
            .iter()
            .filter(|c| node_type(c) == "media")
            .map(media_ref)
            .collect::<Vec<_>>();
        if refs.is_empty() {
            return "[attachment]".into();
        }
        return refs.join("\n");
    }
    // `mediaSingle` wraps the `media` node that carries the file's name.
    let inner = if node_type(node) == "media" {
        node
    } else {
        children(node)
            .iter()
            .find(|c| node_type(c) == "media")
            .unwrap_or(node)
    };
    match attr_str(inner, "alt").map(str::trim) {
        Some(alt)
            if !alt.is_empty()
                && !alt.chars().any(|c| c == '[' || c == ']' || c == '\n') =>
        {
            format!("![{alt}]")
        }
        _ => "[attachment]".into(),
    }
}

fn block_children(node: &Value) -> Vec<String> {
    children(node)
        .iter()
        .map(render_block)
        .filter(|s| !s.trim().is_empty())
        .collect()
}

fn inline_children(node: &Value) -> String {
    children(node).iter().map(|n| render_inline(n, false)).collect()
}

fn rich_inline_children(node: &Value) -> String {
    children(node).iter().map(|n| render_inline(n, true)).collect()
}

fn render_block(node: &Value) -> String {
    let t = node_type(node);
    match t {
        "paragraph" | "heading" | "caption" | "codeBlock" => inline_children(node),
        "bulletList" => list(node, |_, _| "- ".into()),
        "orderedList" => {
            let start = node
                .pointer("/attrs/order")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            list(node, |i, _| format!("{}. ", start + i as u64))
        }
        "taskList" => list(node, |_, item| {
            if attr_str(item, "state") == Some("DONE") {
                "[x] ".into()
            } else {
                "[ ] ".into()
            }
        }),
        "blockquote" => block_children(node)
            .join("\n")
            .lines()
            .map(|l| format!("> {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "rule" => "---".into(),
        "table" => children(node)
            .iter()
            .map(|row| {
                children(row)
                    .iter()
                    .map(|cell| block_children(cell).join(" "))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "media" | "mediaSingle" | "mediaGroup" => media_ref(node),
        _ if is_inline(t) => render_inline(node, false),
        // "doc", "listItem", "panel", "expand", table cells and node types we have never
        // heard of: render what is inside.
        _ => {
            let kids = children(node);
            if kids.is_empty() {
                fallback_text(node)
            } else if kids.iter().all(|k| is_inline(node_type(k))) {
                inline_children(node)
            } else {
                block_children(node).join("\n")
            }
        }
    }
}

fn list(node: &Value, marker: impl Fn(usize, &Value) -> String) -> String {
    children(node)
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = marker(i, item);
            let inner = block_children(item).join("\n");
            let pad = " ".repeat(marker.chars().count());
            let mut lines = inner.lines();
            let mut out = format!("{marker}{}", lines.next().unwrap_or(""));
            for l in lines {
                out.push('\n');
                out.push_str(&pad);
                out.push_str(l);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn fallback_text(node: &Value) -> String {
    node.get("text")
        .and_then(Value::as_str)
        .or_else(|| attr_str(node, "text"))
        .unwrap_or("")
        .into()
}

/// `rich` writes a mention as `@[Name|account id]` (what the views highlight); plain text writes `@Name`.
fn render_inline(node: &Value, rich: bool) -> String {
    match node_type(node) {
        "text" => {
            let text = node.get("text").and_then(Value::as_str).unwrap_or("");
            let href = node
                .get("marks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|m| node_type(m) == "link")
                .and_then(|m| attr_str(m, "href"));
            match href {
                Some(h) if !h.is_empty() && h != text => format!("{text} ({h})"),
                _ => text.into(),
            }
        }
        "hardBreak" => "\n".into(),
        "mention" => {
            let name = attr_str(node, "text").map(|t| t.trim_start_matches('@'));
            match (name, attr_str(node, "id")) {
                // The account id rides along so a view can tell which mention is the reader.
                (Some(n), Some(id)) if !n.is_empty() && rich => format!("@[{n}|{id}]"),
                (Some(n), _) if !n.is_empty() && rich => format!("@[{n}]"),
                (Some(n), _) if !n.is_empty() => format!("@{n}"),
                (_, Some(id)) if rich => format!("@[{id}]"),
                (_, Some(id)) => format!("@{id}"),
                _ => "@".into(),
            }
        }
        "emoji" => attr_str(node, "text")
            .or_else(|| attr_str(node, "shortName"))
            .unwrap_or("")
            .into(),
        "inlineCard" => attr_str(node, "url").unwrap_or("").into(),
        "status" => attr_str(node, "text").unwrap_or("").into(),
        "date" => attr_str(node, "timestamp").unwrap_or("").into(),
        "media" | "mediaInline" => media_ref(node),
        _ => {
            let inner: String = children(node).iter().map(|n| render_inline(n, rich)).collect();
            if inner.is_empty() {
                fallback_text(node)
            } else {
                inner
            }
        }
    }
}

// ---- rich text ------------------------------------------------------------------------

/// A body as light markup the views turn into blocks: paragraphs and other blocks separated by a blank line,
/// `#` headings, `- ` and `1. ` lists (`- [ ]` tasks, nested items indented two spaces), fenced code,
/// `> ` quotes, `@[Name|account id]` mentions and `![file]` references to attached files (a bare
/// `[attachment]` when the body does not say which file). Unlike [`body_to_text`] it keeps the block structure.
pub fn body_to_rich(body: &Value) -> String {
    match body {
        Value::Null => String::new(),
        Value::String(s) => {
            let trimmed = s.trim_start();
            if trimmed.starts_with('{')
                && let Ok(doc) = serde_json::from_str::<Value>(trimmed)
                && doc.get("type").is_some()
            {
                return adf_to_rich(&doc);
            }
            string_body(s).0
        }
        Value::Object(_) => adf_to_rich(body),
        other => other.to_string(),
    }
}

pub fn adf_to_rich(node: &Value) -> String {
    rich_blocks(node).join("\n\n").trim().to_string()
}

/// The blocks inside `node`. A node whose children are all inline is one paragraph.
fn rich_blocks(node: &Value) -> Vec<String> {
    let kids = children(node);
    if !kids.is_empty() && kids.iter().all(|k| is_inline(node_type(k))) {
        return vec![rich_inline_children(node)];
    }
    kids.iter()
        .map(rich_block)
        .filter(|s| !s.trim().is_empty())
        .collect()
}

fn rich_block(node: &Value) -> String {
    let t = node_type(node);
    match t {
        "paragraph" | "caption" => rich_inline_children(node),
        "heading" => {
            let level = node
                .pointer("/attrs/level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), rich_inline_children(node))
        }
        "codeBlock" => format!("```\n{}\n```", inline_children(node)),
        "bulletList" => rich_list(node, |_, _| "- ".into()),
        "orderedList" => {
            let start = node
                .pointer("/attrs/order")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            rich_list(node, |i, _| format!("{}. ", start + i as u64))
        }
        "taskList" => rich_list(node, |_, item| {
            if attr_str(item, "state") == Some("DONE") {
                "- [x] ".into()
            } else {
                "- [ ] ".into()
            }
        }),
        "blockquote" => rich_blocks(node)
            .join("\n\n")
            .lines()
            .map(|l| {
                if l.is_empty() {
                    ">".to_string()
                } else {
                    format!("> {l}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "rule" => "---".into(),
        "table" => children(node)
            .iter()
            .map(|row| {
                let cells: Vec<String> = children(row)
                    .iter()
                    .map(|cell| rich_blocks(cell).join(" "))
                    .collect();
                format!("| {} |", cells.join(" | "))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "media" | "mediaSingle" | "mediaGroup" => media_ref(node),
        _ if is_inline(t) => render_inline(node, true),
        _ => {
            let kids = children(node);
            if kids.is_empty() {
                fallback_text(node)
            } else {
                rich_blocks(node).join("\n\n")
            }
        }
    }
}

/// A list: each item's first block on the marker's line, anything after it (another paragraph, a nested list)
/// indented under it.
fn rich_list(node: &Value, marker: impl Fn(usize, &Value) -> String) -> String {
    children(node)
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = marker(i, item);
            let inner = rich_blocks(item).join("\n");
            let pad = "  ";
            let mut lines = inner.lines();
            let mut out = format!("{marker}{}", lines.next().unwrap_or(""));
            for l in lines {
                out.push('\n');
                out.push_str(pad);
                out.push_str(l);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rich_text_keeps_the_block_structure() {
        let doc = json!({"type":"doc","version":1,"content":[
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Plan"}]},
            {"type":"paragraph","content":[
                {"type":"text","text":"Ask "},
                {"type":"mention","attrs":{"id":"a1","text":"@Ada"}},
                {"type":"hardBreak"},
                {"type":"text","text":"second line"}
            ]},
            {"type":"bulletList","content":[
                {"type":"listItem","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"one"}]},
                    {"type":"bulletList","content":[{"type":"listItem","content":[
                        {"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}
                ]},
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
            ]},
            {"type":"orderedList","attrs":{"order":3},"content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"third"}]}]}
            ]},
            {"type":"taskList","content":[
                {"type":"taskItem","attrs":{"state":"DONE"},"content":[{"type":"text","text":"done"}]},
                {"type":"taskItem","attrs":{"state":"TODO"},"content":[{"type":"text","text":"todo"}]}
            ]},
            {"type":"codeBlock","content":[{"type":"text","text":"let x = 1;"}]},
            {"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"quoted"}]}]},
            {"type":"paragraph","content":[{"type":"text","text":"End"}]}
        ]});
        assert_eq!(
            adf_to_rich(&doc),
            "## Plan\n\nAsk @[Ada|a1]\nsecond line\n\n- one\n  - nested\n- two\n\n3. third\n\n- [x] done\n- [ ] todo\n\n```\nlet x = 1;\n```\n\n> quoted\n\nEnd"
        );
        // The plain rendering is unchanged: no headings marks, mentions as `@Name`.
        assert!(adf_to_text(&doc).starts_with("Plan\nAsk @Ada"));
    }

    /// A file dropped into the body reads as a reference naming it, wherever it sits: on its own
    /// line, mid-paragraph, or in a list. A node that does not say which file stays the bare
    /// placeholder, so nothing claims a name it was never given.
    #[test]
    fn attached_files_read_as_references_where_they_sit() {
        let media = |alt: Option<&str>| {
            let mut attrs = serde_json::Map::new();
            attrs.insert("id".into(), json!("media-id-1"));
            attrs.insert("type".into(), json!("file"));
            if let Some(alt) = alt {
                attrs.insert("alt".into(), json!(alt));
            }
            json!({"type":"media","attrs":Value::Object(attrs)})
        };
        let doc = json!({"type":"doc","version":1,"content":[
            {"type":"paragraph","content":[
                {"type":"text","text":"See "},
                {"type":"mediaInline","attrs":{"id":"media-id-2","type":"file","alt":"shot.png"}},
                {"type":"text","text":" for the glitch."}
            ]},
            {"type":"mediaSingle","content":[media(Some("trace.har"))]},
            {"type":"mediaGroup","content":[media(None), media(Some("a]b.png"))]},
        ]});
        assert_eq!(
            adf_to_rich(&doc),
            "See ![shot.png] for the glitch.\n\n![trace.har]\n\n[attachment]\n[attachment]"
        );
        // The plain rendering names the file too: it reads as well in a notification as here.
        assert_eq!(
            adf_to_text(&doc),
            "See ![shot.png] for the glitch.\n![trace.har]\n[attachment]\n[attachment]"
        );
    }

    #[test]
    fn paragraphs_marks_links_breaks_and_mentions() {
        let doc = json!({"type":"doc","version":1,"content":[
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Title"}]},
            {"type":"paragraph","content":[
                {"type":"mention","attrs":{"id":"acc-1","text":"@Ada Lovelace"}},
                {"type":"text","text":" please look at "},
                {"type":"text","text":"this","marks":[{"type":"link","attrs":{"href":"https://x.test/a"}}]},
                {"type":"hardBreak"},
                {"type":"text","text":"bold","marks":[{"type":"strong"}]},
                {"type":"mention","attrs":{"id":"acc-1","text":"@Ada Lovelace"}},
                {"type":"mention","attrs":{"id":"acc-2"}}
            ]}
        ]});
        let (text, mentions) = body_to_text(&doc);
        assert_eq!(
            text,
            "Title\n@Ada Lovelace please look at this (https://x.test/a)\nbold@Ada Lovelace@acc-2"
        );
        assert_eq!(mentions, ["acc-1", "acc-2"]);
    }

    #[test]
    fn nested_lists_code_blocks_and_quotes() {
        let doc = json!({"type":"doc","content":[
            {"type":"bulletList","content":[
                {"type":"listItem","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"one"}]},
                    {"type":"orderedList","attrs":{"order":3},"content":[
                        {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"deep"}]}]}
                    ]}
                ]},
                {"type":"listItem","content":[{"type":"paragraph","content":[
                    {"type":"mention","attrs":{"id":"acc-9","text":"Bob"}}]}]}
            ]},
            {"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"let x = 1;\nlet y = 2;"}]},
            {"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"quoted"}]}]}
        ]});
        let (text, mentions) = body_to_text(&doc);
        assert_eq!(
            text,
            "- one\n  3. deep\n- @Bob\nlet x = 1;\nlet y = 2;\n> quoted"
        );
        assert_eq!(mentions, ["acc-9"]);
    }

    #[test]
    fn unknown_nodes_never_fail_and_mentions_inside_them_are_found() {
        let doc = json!({"type":"doc","content":[
            {"type":"futureWidget","attrs":{"x":1},"content":[
                {"type":"paragraph","content":[
                    {"type":"mention","attrs":{"id":"acc-7","text":"@Zed"}},
                    {"type":"text","text":" hi"}]}]},
            {"type":"alienLeaf","text":"alien"},
            {"type":"panel","content":[{"type":"paragraph","content":[{"type":"text","text":"in panel"}]}]},
            {"type":"table","content":[{"type":"tableRow","content":[
                {"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]},
                {"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"b"}]}]}]}]}
        ]});
        let (text, mentions) = body_to_text(&doc);
        assert_eq!(text, "@Zed hi\nalien\nin panel\na | b");
        assert_eq!(mentions, ["acc-7"]);
    }

    #[test]
    fn stringified_adf_plain_text_wiki_and_html() {
        let adf = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"mention","attrs":{"id":"a1","text":"@A"}}]}]}"#;
        let (t, m) = body_to_text(&Value::String(adf.into()));
        assert_eq!((t.as_str(), m), ("@A", vec!["a1".to_string()]));

        let (t, m) = body_to_text(&json!("just words { not json"));
        assert_eq!((t.as_str(), m.len()), ("just words { not json", 0));

        let (t, m) = body_to_text(&json!(
            "hi [~accountid:5b10a] and [~accountid:5b10a] [~accountid:zz9]!"
        ));
        assert_eq!(t, "hi @5b10a and @5b10a @zz9!");
        assert_eq!(m, ["5b10a", "zz9"]);

        let (_, m) = body_to_text(&json!("[~accountid: broken and [~accountid:]"));
        assert!(m.is_empty());

        let (_, m) = body_to_text(&json!(
            r#"<p><a href="x" data-account-id="abc123" class="user-hover">@A</a></p>"#
        ));
        assert_eq!(m, ["abc123"]);

        assert_eq!(body_to_text(&Value::Null), (String::new(), vec![]));
        assert_eq!(body_to_text(&json!(42)).0, "42");
        assert_eq!(body_to_text(&json!({"type":"doc","content":[]})).0, "");
    }
}
