//! What `acli ... --json` prints, as tolerant serde structs, and the conversions to the
//! provider model.
//!
//! Verified against a real `acli 1.3.39-stable`: `search` prints a top-level JSON array of
//! Jira-REST-like issues (`key`, `fields{summary,status,priority,assignee}` plus many keys
//! that are `null`); `view` prints one such object; `view --fields comment` nests the
//! comments under `fields.comment` with author account ids, ADF bodies and Jira
//! timestamps. The parsers stay tolerant (unknown fields, flat fields, an object wrapping
//! the array, several documents in one output, string-or-object status/priority/assignee).

use serde::Deserialize;
use serde_json::Value;

use super::UPDATED_UNKNOWN;
use super::adf::body_to_text;
use super::time::parse_timestamp;
use crate::domain::TicketKey;
use crate::providers::error::{ProviderError, ProviderResult};
use crate::providers::model::{
    Attachment, IssueLink, RemoteComment, RemoteTicket, Subtask, TicketDetail,
};

pub const TOOL: &str = "acli";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WireFields {
    summary: Option<String>,
    status: Option<Value>,
    priority: Option<Value>,
    assignee: Option<Value>,
    updated: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WireIssue {
    key: String,
    #[serde(rename = "self")]
    self_url: Option<String>,
    fields: WireFields,
    /// Fallback for output that is not nested under `fields`.
    #[serde(flatten)]
    flat: WireFields,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WireComment {
    id: Value,
    author: Value,
    #[serde(rename = "updateAuthor")]
    update_author: Value,
    body: Value,
    created: Value,
}

/// `s` cut to at most `max` characters, for error messages.
pub fn snippet(s: &str, max: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(max) {
        Some((cut, _)) => format!("{}...", &s[..cut]),
        None => s.into(),
    }
}

pub fn parse_error(what: &str, detail: impl std::fmt::Display, output: &str) -> ProviderError {
    ProviderError::Parse {
        tool: TOOL.into(),
        what: what.into(),
        detail: format!("{detail}; output began: {:?}", snippet(output, 300)),
    }
}

/// Every JSON document in `output` (acli may print one per page). Empty output is no
/// documents.
pub fn json_documents(output: &str, what: &str) -> ProviderResult<Vec<Value>> {
    serde_json::Deserializer::from_str(output)
        .into_iter::<Value>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| parse_error(what, e, output))
}

/// The elements of a document that is either an array or an object holding one under one
/// of `keys`.
fn elements(doc: Value, keys: &[&str]) -> Option<Vec<Value>> {
    match doc {
        Value::Array(items) => Some(items),
        Value::Object(mut map) => keys.iter().find_map(|k| match map.remove(*k) {
            Some(Value::Array(items)) => Some(items),
            _ => None,
        }),
        _ => None,
    }
}

/// Tickets of a `search` output (all pages combined).
pub fn parse_search(output: &str, site: Option<&str>) -> ProviderResult<Vec<RemoteTicket>> {
    const WHAT: &str = "search results";
    let mut tickets = Vec::new();
    let documents = json_documents(output, WHAT)?;
    // A search that finds nothing prints `[]` (verified against a real acli). No document at
    // all is a run that did not answer; taking it for "no tickets" would make sync drop
    // everything cached.
    if documents.is_empty() {
        return Err(parse_error(WHAT, "empty output", output));
    }
    for doc in documents {
        let items = if doc.get("key").is_some() {
            vec![doc]
        } else {
            elements(
                doc,
                &["issues", "workItems", "workitems", "values", "results"],
            )
            .ok_or_else(|| parse_error(WHAT, "expected an array of work items", output))?
        };
        for item in items {
            tickets.push(ticket_from(item, site, WHAT, output)?);
        }
    }
    Ok(tickets)
}

/// The single ticket of a `view` output; `Ok(None)` when the output holds none.
pub fn parse_view(output: &str, site: Option<&str>) -> ProviderResult<Option<RemoteTicket>> {
    const WHAT: &str = "work item";
    let Some(doc) = json_documents(output, WHAT)?.into_iter().next() else {
        return Ok(None);
    };
    let item = match doc {
        Value::Array(items) => match items.into_iter().next() {
            Some(first) => first,
            None => return Ok(None),
        },
        other => other,
    };
    if !item.is_object() {
        return Err(parse_error(WHAT, "expected a JSON object", output));
    }
    ticket_from(item, site, WHAT, output).map(Some)
}

fn name_of(value: &Option<Value>) -> Option<String> {
    match value.as_ref()? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(o) => ["name", "displayName", "value", "emailAddress"]
            .iter()
            .find_map(|k| o.get(*k).and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .map(String::from),
        _ => None,
    }
}

fn timestamp_of(value: &Value, what: &str, output: &str) -> ProviderResult<i64> {
    match value {
        Value::Null => Ok(0),
        Value::String(s) => parse_timestamp(s)
            .ok_or_else(|| parse_error(what, format!("unrecognised timestamp {s:?}"), output)),
        Value::Number(n) => n
            .as_i64()
            .and_then(|n| parse_timestamp(&n.to_string()))
            .ok_or_else(|| parse_error(what, format!("unrecognised timestamp {n}"), output)),
        other => Err(parse_error(
            what,
            format!("unrecognised timestamp {other}"),
            output,
        )),
    }
}

fn browse_url(key: &TicketKey, site: Option<&str>, self_url: Option<&str>) -> Option<String> {
    let host = match site {
        Some(s) if !s.trim().is_empty() => s
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string(),
        _ => {
            let rest = self_url?.strip_prefix("https://")?;
            rest.split('/')
                .next()
                .filter(|h| !h.is_empty())?
                .to_string()
        }
    };
    Some(format!("https://{host}/browse/{key}"))
}

fn ticket_from(
    item: Value,
    site: Option<&str>,
    what: &str,
    output: &str,
) -> ProviderResult<RemoteTicket> {
    let issue: WireIssue =
        serde_json::from_value(item).map_err(|e| parse_error(what, e, output))?;
    let key: TicketKey = issue
        .key
        .parse()
        .map_err(|e| parse_error(what, format!("{e}"), output))?;
    let f = &issue.fields;
    let flat = &issue.flat;
    let title = f
        .summary
        .clone()
        .or_else(|| flat.summary.clone())
        .unwrap_or_default();
    let status = name_of(&f.status)
        .or_else(|| name_of(&flat.status))
        .unwrap_or_default();
    let updated = f.updated.as_ref().or(flat.updated.as_ref());
    let updated_at = match updated {
        Some(v) => timestamp_of(v, what, output)?,
        None => UPDATED_UNKNOWN,
    };
    Ok(RemoteTicket {
        url: browse_url(&key, site, issue.self_url.as_deref()),
        key,
        title,
        status,
        priority: name_of(&f.priority).or_else(|| name_of(&flat.priority)),
        assignee: name_of(&f.assignee).or_else(|| name_of(&flat.assignee)),
        updated_at,
    })
}

/// Comments of a `view --fields comment` output, oldest first, plus the `total` Jira
/// reports (so the caller can tell a truncated list). Shape:
/// `{"fields": {"comment": {"comments": [...], "total": N, "maxResults": M, "startAt": 0}}}`.
/// A `null` or missing comment field is no comments.
pub fn parse_comment_field(
    output: &str,
    ticket: &TicketKey,
) -> ProviderResult<(Vec<RemoteComment>, Option<usize>)> {
    const WHAT: &str = "comments";
    let docs = json_documents(output, WHAT)?;
    let Some(doc) = docs.into_iter().next() else {
        return Err(parse_error(WHAT, "empty output", output));
    };
    let doc = match doc {
        Value::Array(items) => items.into_iter().next().unwrap_or(Value::Null),
        other => other,
    };
    // Refuse other shapes (notably the flat `comment list` output, which has no `fields`)
    // instead of reading them as "no comments".
    if !doc.get("fields").is_some_and(Value::is_object) {
        return Err(parse_error(
            WHAT,
            "expected a work item object with `fields`",
            output,
        ));
    }
    let container = doc
        .pointer("/fields/comment")
        .cloned()
        .unwrap_or(Value::Null);
    let total = container
        .get("total")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok());
    let items = match container {
        Value::Null => Vec::new(),
        Value::Object(mut map) => match map.remove("comments") {
            Some(Value::Array(items)) => items,
            Some(Value::Null) | None => Vec::new(),
            Some(_) => return Err(parse_error(WHAT, "`comments` is not an array", output)),
        },
        _ => {
            return Err(parse_error(
                WHAT,
                "`fields.comment` is not an object",
                output,
            ));
        }
    };
    let mut comments = Vec::with_capacity(items.len());
    for item in items {
        comments.push(comment_from(item, ticket, WHAT, output)?);
    }
    // Stable: comments with equal timestamps keep the order acli gave.
    comments.sort_by_key(|c| c.created_at);
    Ok((comments, total))
}

/// A `view --fields '*all'` output: the ticket's comments (as [`parse_comment_field`]) and its detail. The
/// detail is read leniently: a field that is missing or in a shape we do not know is left empty, so it can
/// never cost the comments.
pub fn parse_view_all(
    output: &str,
    ticket: &TicketKey,
) -> ProviderResult<(TicketDetail, Vec<RemoteComment>, Option<usize>)> {
    let (comments, total) = parse_comment_field(output, ticket)?;
    let doc = json_documents(output, "ticket detail")?
        .into_iter()
        .next()
        .map(|d| match d {
            Value::Array(items) => items.into_iter().next().unwrap_or(Value::Null),
            other => other,
        })
        .unwrap_or(Value::Null);
    Ok((detail_from(&doc), comments, total))
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn names_at(f: &Value, field: &str) -> Vec<String> {
    f.get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.get("name").and_then(Value::as_str).or(i.as_str()))
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// The sprint field is a custom field whose id differs per site, so it is found by its shape: an array of
/// objects with a `boardId` and a `name`. The active sprint wins, else the newest (last) one.
fn sprint_of(fields: &Value) -> Option<String> {
    let map = fields.as_object()?;
    map.values().find_map(|v| {
        let sprints: Vec<&Value> = v
            .as_array()?
            .iter()
            .filter(|s| s.get("boardId").is_some() && s.get("name").is_some())
            .collect();
        let chosen = sprints
            .iter()
            .find(|s| s.get("state").and_then(Value::as_str) == Some("active"))
            .or(sprints.last())?;
        chosen.get("name")?.as_str().map(String::from)
    })
}

fn detail_from(doc: &Value) -> TicketDetail {
    let f = doc.get("fields").unwrap_or(&Value::Null);
    let time = |field: &str| {
        f.get(field)
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
    };
    TicketDetail {
        issue_type: str_at(f, "/issuetype/name").map(String::from),
        reporter: str_at(f, "/reporter/displayName")
            .or_else(|| str_at(f, "/reporter/name"))
            .map(String::from),
        created_at: time("created"),
        updated_at: time("updated"),
        labels: f
            .get("labels")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        components: names_at(f, "components"),
        fix_versions: names_at(f, "fixVersions"),
        sprint: sprint_of(f),
        parent: doc
            .pointer("/fields/parent")
            .or_else(|| f.get("parent"))
            .and_then(|p| {
                let key = p.get("key")?.as_str()?;
                let title = str_at(p, "/fields/summary").unwrap_or_default();
                Some((key.to_string(), title.to_string()))
            }),
        original_estimate: str_at(f, "/timetracking/originalEstimate").map(String::from),
        description: f
            .get("description")
            .map(super::adf::body_to_rich)
            .unwrap_or_default(),
        attachments: f
            .get("attachment")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|a| {
                        Some(Attachment {
                            name: a.get("filename")?.as_str()?.to_string(),
                            bytes: a.get("size").and_then(Value::as_u64).unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        links: f
            .get("issuelinks")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(link_from).collect())
            .unwrap_or_default(),
        subtasks: f
            .get("subtasks")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|s| {
                        Some(Subtask {
                            key: s.get("key")?.as_str()?.parse().ok()?,
                            title: str_at(s, "/fields/summary")
                                .unwrap_or_default()
                                .to_string(),
                            done: str_at(s, "/fields/status/statusCategory/key") == Some("done"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn link_from(link: &Value) -> Option<IssueLink> {
    let (relation, issue) = match (link.get("outwardIssue"), link.get("inwardIssue")) {
        (Some(issue), _) => (str_at(link, "/type/outward")?, issue),
        (None, Some(issue)) => (str_at(link, "/type/inward")?, issue),
        (None, None) => return None,
    };
    Some(IssueLink {
        relation: relation.to_string(),
        key: issue.get("key")?.as_str()?.parse().ok()?,
        title: str_at(issue, "/fields/summary").unwrap_or_default().to_string(),
        status: str_at(issue, "/fields/status/name")
            .unwrap_or_default()
            .to_string(),
    })
}

/// One comment object (the result of `comment create`).
pub fn parse_created_comment(
    output: &str,
    ticket: &TicketKey,
) -> ProviderResult<Option<RemoteComment>> {
    const WHAT: &str = "created comment";
    let Some(doc) = json_documents(output, WHAT)?.into_iter().next() else {
        return Ok(None);
    };
    let item = match doc {
        Value::Array(items) => items.into_iter().next(),
        Value::Object(mut map) => match map.remove("comment") {
            Some(inner @ Value::Object(_)) => Some(inner),
            _ => Some(Value::Object(map)),
        },
        _ => None,
    };
    match item {
        Some(item) if item.get("id").is_some() => {
            comment_from(item, ticket, WHAT, output).map(Some)
        }
        _ => Ok(None),
    }
}

fn user_of(value: &Value) -> (String, String) {
    match value {
        Value::Object(o) => {
            let get = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            (get("accountId"), get("displayName"))
        }
        Value::String(s) => (String::new(), s.clone()),
        _ => (String::new(), String::new()),
    }
}

fn comment_from(
    item: Value,
    ticket: &TicketKey,
    what: &str,
    output: &str,
) -> ProviderResult<RemoteComment> {
    let c: WireComment = serde_json::from_value(item).map_err(|e| parse_error(what, e, output))?;
    let id = match &c.id {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(parse_error(what, "comment without an id", output)),
    };
    let (account, name) = match user_of(&c.author) {
        (a, n) if !a.is_empty() || !n.is_empty() => (a, n),
        _ => user_of(&c.update_author),
    };
    let (body_text, mentions) = body_to_text(&c.body);
    let rich = super::adf::body_to_rich(&c.body);
    Ok(RemoteComment {
        id,
        ticket: ticket.clone(),
        author_account_id: account,
        author_name: name,
        body_text,
        rich,
        mentions,
        created_at: timestamp_of(&c.created, what, output)?,
    })
}
