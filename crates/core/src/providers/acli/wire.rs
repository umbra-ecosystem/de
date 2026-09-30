//! What `acli ... --json` prints, as tolerant serde structs, and the conversions to the
//! provider model.
//!
//! UNVERIFIED: Atlassian's ACLI documentation does not show the JSON shape of `view`,
//! `search` or `comment list`. These structs assume the Jira REST shape (`key`, `fields`
//! with `summary`/`status`/`priority`/`assignee`/`updated`; comments with `id`, `author`,
//! `body`, `created`) and tolerate: unknown fields, flat (un-nested) fields, a bare array or
//! an object wrapping the array, several JSON documents in one output (one per page), and
//! status/priority/assignee as either a string or an object. `de providers probe` output
//! is how to verify this on a real install.

use serde::Deserialize;
use serde_json::Value;

use super::adf::body_to_text;
use super::time::parse_timestamp;
use crate::domain::TicketKey;
use crate::providers::error::{ProviderError, ProviderResult};
use crate::providers::model::{RemoteComment, RemoteTicket};

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
    for doc in json_documents(output, WHAT)? {
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
            rest.split('/').next().filter(|h| !h.is_empty())?.to_string()
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
        None => 0,
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

/// Comments of a `comment list` output, oldest first.
pub fn parse_comments(output: &str, ticket: &TicketKey) -> ProviderResult<Vec<RemoteComment>> {
    const WHAT: &str = "comments";
    let mut comments = Vec::new();
    for doc in json_documents(output, WHAT)? {
        let items = elements(doc, &["comments", "values", "results"])
            .ok_or_else(|| parse_error(WHAT, "expected an array of comments", output))?;
        for item in items {
            comments.push(comment_from(item, ticket, WHAT, output)?);
        }
    }
    // Stable: comments with equal timestamps keep the order acli gave.
    comments.sort_by_key(|c| c.created_at);
    Ok(comments)
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
    let c: WireComment =
        serde_json::from_value(item).map_err(|e| parse_error(what, e, output))?;
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
    Ok(RemoteComment {
        id,
        ticket: ticket.clone(),
        author_account_id: account,
        author_name: name,
        body_text,
        mentions,
        created_at: timestamp_of(&c.created, what, output)?,
    })
}
