//! Tolerant wire structs for what `bkt` prints, and their conversion to the model.
//!
//! Every struct is `#[serde(default)]` with null-tolerant fields: Go's encoder and the
//! Bitbucket API emit `null` freely, and unknown fields are ignored.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::providers::{
    DiffSide, InlineAnchor, PipelineRun, PipelineState, PipelineStep, Pr, PrComment, PrState,
    Reviewer,
};

fn nz<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WUser {
    #[serde(deserialize_with = "nz")]
    pub uuid: String,
    #[serde(deserialize_with = "nz")]
    pub account_id: String,
    #[serde(deserialize_with = "nz")]
    pub nickname: String,
    #[serde(deserialize_with = "nz")]
    pub username: String,
    #[serde(deserialize_with = "nz")]
    pub display_name: String,
}

impl WUser {
    /// Stable identity used as `Reviewer::account` / `PrComment::author`: Atlassian
    /// account id, then nickname, username, uuid, display name.
    pub fn ident(&self) -> String {
        [
            &self.account_id,
            &self.nickname,
            &self.username,
            &self.uuid,
            &self.display_name,
        ]
        .into_iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default()
    }

    fn same_as(&self, other: &WUser) -> bool {
        (!self.account_id.is_empty() && self.account_id == other.account_id)
            || (!self.uuid.is_empty() && self.uuid == other.uuid)
            || (!self.nickname.is_empty() && self.nickname == other.nickname)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WBranch {
    #[serde(deserialize_with = "nz")]
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WHash {
    #[serde(deserialize_with = "nz")]
    pub hash: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WEnd {
    #[serde(deserialize_with = "nz")]
    pub branch: WBranch,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WHref {
    #[serde(deserialize_with = "nz")]
    pub href: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WLinks {
    #[serde(deserialize_with = "nz")]
    pub html: WHref,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WParticipant {
    #[serde(deserialize_with = "nz")]
    pub user: WUser,
    #[serde(deserialize_with = "nz")]
    pub role: String,
    pub state: Option<String>,
    pub approved: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WPr {
    #[serde(deserialize_with = "nz")]
    pub id: u64,
    #[serde(deserialize_with = "nz")]
    pub title: String,
    #[serde(deserialize_with = "nz")]
    pub state: String,
    #[serde(deserialize_with = "nz")]
    pub updated_on: String,
    #[serde(deserialize_with = "nz")]
    pub created_on: String,
    #[serde(deserialize_with = "nz")]
    pub author: WUser,
    #[serde(deserialize_with = "nz")]
    pub source: WEnd,
    #[serde(deserialize_with = "nz")]
    pub destination: WEnd,
    #[serde(deserialize_with = "nz")]
    pub links: WLinks,
    #[serde(deserialize_with = "nz")]
    pub reviewers: Vec<WUser>,
    #[serde(deserialize_with = "nz")]
    pub participants: Vec<WParticipant>,
}

pub fn pr_state(text: &str) -> Option<PrState> {
    PrState::parse(&text.trim().to_ascii_lowercase())
}

impl WPr {
    pub fn into_pr(self, repo: &str) -> Result<Pr, String> {
        if self.id == 0 {
            return Err("pull request without an id".into());
        }
        let state = pr_state(&self.state)
            .ok_or_else(|| format!("unknown pull request state {:?}", self.state))?;
        let reviewers = reviewers(&self.reviewers, &self.participants);
        Ok(Pr {
            repo: repo.into(),
            id: self.id,
            title: self.title,
            state,
            source_branch: self.source.branch.name,
            destination_branch: self.destination.branch.name,
            author: self.author.ident(),
            reviewers,
            url: self.links.html.href,
            updated_at: parse_rfc3339(&self.updated_on)
                .or_else(|| parse_rfc3339(&self.created_on))
                .unwrap_or(0),
        })
    }
}

/// Reviewers come from `reviewers` (identities only) overlaid with `participants` (approval
/// state). A participant who is not a listed reviewer is included only when they approved
/// or requested changes.
fn reviewers(listed: &[WUser], participants: &[WParticipant]) -> Vec<Reviewer> {
    let flags = |p: &WParticipant| {
        let state = p.state.as_deref().unwrap_or("").to_ascii_lowercase();
        (
            p.approved.unwrap_or(false) || state == "approved",
            state == "changes_requested",
        )
    };
    let mut out: Vec<Reviewer> = listed
        .iter()
        .map(|u| {
            let (approved, changes_requested) = participants
                .iter()
                .find(|p| p.user.same_as(u))
                .map(flags)
                .unwrap_or((false, false));
            Reviewer {
                account: u.ident(),
                approved,
                changes_requested,
            }
        })
        .collect();
    for p in participants {
        if listed.iter().any(|u| u.same_as(&p.user)) {
            continue;
        }
        let (approved, changes_requested) = flags(p);
        if p.role.eq_ignore_ascii_case("REVIEWER") || approved || changes_requested {
            out.push(Reviewer {
                account: p.user.ident(),
                approved,
                changes_requested,
            });
        }
    }
    out
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WRaw {
    #[serde(deserialize_with = "nz")]
    pub raw: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WInline {
    #[serde(deserialize_with = "nz")]
    pub path: String,
    pub from: Option<u32>,
    pub to: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WComment {
    #[serde(deserialize_with = "nz")]
    pub id: u64,
    #[serde(deserialize_with = "nz")]
    pub content: WRaw,
    #[serde(deserialize_with = "nz")]
    pub user: WUser,
    #[serde(deserialize_with = "nz")]
    pub created_on: String,
    #[serde(deserialize_with = "nz")]
    pub deleted: bool,
    pub inline: Option<WInline>,
}

impl WComment {
    pub fn into_comment(self, pr: u64) -> Result<PrComment, String> {
        if self.id == 0 {
            return Err("comment without an id".into());
        }
        let inline = self.inline.and_then(|i| {
            // `to` is the new-file line, `from` the old-file line.
            match (i.to, i.from) {
                (Some(line), _) => Some((i.path, line, DiffSide::New)),
                (None, Some(line)) => Some((i.path, line, DiffSide::Old)),
                (None, None) => None,
            }
        });
        Ok(PrComment {
            id: self.id,
            pr,
            author: self.user.ident(),
            body: self.content.raw,
            inline: inline.map(|(path, line, side)| InlineAnchor { path, line, side }),
            created_at: parse_rfc3339(&self.created_on).unwrap_or(0),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WName {
    #[serde(deserialize_with = "nz")]
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WState {
    #[serde(deserialize_with = "nz")]
    pub name: String,
    #[serde(deserialize_with = "nz")]
    pub result: WName,
    #[serde(deserialize_with = "nz")]
    pub stage: WName,
}

/// Maps Bitbucket pipeline/step states. Table (see tests):
/// `PENDING`->Pending; `IN_PROGRESS`->Running (stage `PAUSED`/`HALTED` -> Other);
/// `COMPLETED` + `SUCCESSFUL`->Succeeded, `FAILED`/`ERROR`/`EXPIRED`->Failed, `STOPPED`->Stopped;
/// anything else (`NOT_RUN`, `SKIPPED`, unknown) -> Other(original text).
pub fn map_state(s: &WState) -> PipelineState {
    let name = s.name.trim().to_ascii_uppercase();
    let result = s.result.name.trim().to_ascii_uppercase();
    let stage = s.stage.name.trim().to_ascii_uppercase();
    match name.as_str() {
        "PENDING" => PipelineState::Pending,
        "IN_PROGRESS" => match stage.as_str() {
            "PAUSED" | "HALTED" => PipelineState::Other(stage),
            _ => PipelineState::Running,
        },
        "COMPLETED" => match result.as_str() {
            "SUCCESSFUL" => PipelineState::Succeeded,
            "FAILED" | "ERROR" | "EXPIRED" => PipelineState::Failed,
            "STOPPED" => PipelineState::Stopped,
            "" => PipelineState::Other("COMPLETED".into()),
            other => PipelineState::Other(other.into()),
        },
        // Some bkt/REST variants report the outcome directly as the state name.
        "SUCCESSFUL" => PipelineState::Succeeded,
        "FAILED" | "ERROR" => PipelineState::Failed,
        "STOPPED" => PipelineState::Stopped,
        "" => PipelineState::Other("UNKNOWN".into()),
        other => PipelineState::Other(other.into()),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WSelector {
    #[serde(rename = "type", deserialize_with = "nz")]
    pub kind: String,
    #[serde(deserialize_with = "nz")]
    pub pattern: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WTarget {
    #[serde(rename = "type", deserialize_with = "nz")]
    pub kind: String,
    #[serde(deserialize_with = "nz")]
    pub ref_type: String,
    #[serde(deserialize_with = "nz")]
    pub ref_name: String,
    /// `bkt`'s own struct spells the branch `target.ref.name`; accept both.
    #[serde(rename = "ref", deserialize_with = "nz")]
    pub ref_obj: WName,
    /// Pull-request pipelines name the source branch here.
    #[serde(deserialize_with = "nz")]
    pub source: String,
    #[serde(deserialize_with = "nz")]
    pub commit: WHash,
    #[serde(deserialize_with = "nz")]
    pub selector: WSelector,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WPipeline {
    #[serde(deserialize_with = "nz")]
    pub uuid: String,
    pub build_number: Option<u64>,
    #[serde(deserialize_with = "nz")]
    pub state: WState,
    #[serde(deserialize_with = "nz")]
    pub target: WTarget,
    #[serde(deserialize_with = "nz")]
    pub created_on: String,
    pub completed_on: Option<String>,
}

impl WPipeline {
    pub fn into_run(self, repo: &str) -> Result<PipelineRun, String> {
        if self.uuid.is_empty() {
            return Err("pipeline without a uuid".into());
        }
        let branch = [
            &self.target.ref_name,
            &self.target.ref_obj.name,
            &self.target.source,
        ]
        .into_iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default();
        // UNVERIFIED: the web URL is built from the documented Bitbucket Cloud pattern;
        // the API's pipeline object carries only `links.self` (an API URL).
        let url = match self.build_number {
            Some(n) => format!("https://bitbucket.org/{repo}/pipelines/results/{n}"),
            None => String::new(),
        };
        Ok(PipelineRun {
            repo: repo.into(),
            id: self.uuid,
            number: self.build_number,
            state: map_state(&self.state),
            branch,
            commit: self.target.commit.hash,
            created_at: parse_rfc3339(&self.created_on).unwrap_or(0),
            completed_at: self.completed_on.as_deref().and_then(parse_rfc3339),
            url,
            steps: Vec::new(),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WStep {
    #[serde(deserialize_with = "nz")]
    pub name: String,
    #[serde(deserialize_with = "nz")]
    pub state: WState,
    /// UNVERIFIED shape: an object with `uuid` (and maybe `name`/`slug`), or a string.
    #[serde(deserialize_with = "nz")]
    pub environment: Value,
    /// UNVERIFIED: some payloads nest it as `deployment.environment` or a plain string.
    #[serde(deserialize_with = "nz")]
    pub deployment: Value,
}

/// How a step names its deployment environment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvRef {
    pub name: String,
    pub slug: String,
    pub uuid: String,
}

fn env_from(v: &Value) -> Option<EnvRef> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(EnvRef {
            name: s.trim().into(),
            ..EnvRef::default()
        }),
        Value::Object(m) => {
            let get = |k: &str| {
                m.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            let r = EnvRef {
                name: get("name"),
                slug: get("slug"),
                uuid: get("uuid"),
            };
            (r != EnvRef::default()).then_some(r)
        }
        _ => None,
    }
}

impl WStep {
    pub fn env_ref(&self) -> Option<EnvRef> {
        env_from(&self.environment)
            .or_else(|| self.deployment.get("environment").and_then(env_from))
            .or_else(|| env_from(&self.deployment))
    }

    pub fn into_step(self, environment: Option<String>) -> PipelineStep {
        PipelineStep {
            name: self.name,
            state: map_state(&self.state),
            deployment_environment: environment,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WEnvironment {
    #[serde(deserialize_with = "nz")]
    pub uuid: String,
    #[serde(deserialize_with = "nz")]
    pub name: String,
    #[serde(deserialize_with = "nz")]
    pub slug: String,
}

/// Bitbucket's paged envelope.
#[derive(Debug, Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de>"))]
pub struct WPage<T> {
    #[serde(deserialize_with = "nz")]
    pub values: Vec<T>,
    #[serde(deserialize_with = "nz")]
    pub next: String,
}

impl<T> Default for WPage<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            next: String::new(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WAuthHost {
    #[serde(deserialize_with = "nz")]
    pub key: String,
    #[serde(deserialize_with = "nz")]
    pub kind: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WAuthContext {
    #[serde(deserialize_with = "nz")]
    pub name: String,
    #[serde(deserialize_with = "nz")]
    pub host: String,
    #[serde(deserialize_with = "nz")]
    pub active: bool,
}

/// `bkt auth status --json`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WAuthStatus {
    #[serde(deserialize_with = "nz")]
    pub hosts: Vec<WAuthHost>,
    #[serde(deserialize_with = "nz")]
    pub contexts: Vec<WAuthContext>,
}

/// The first `major.minor.patch` in `text`.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit()
            && (i == 0
                || !bytes[i - 1].is_ascii_alphanumeric()
                || matches!(bytes[i - 1], b'v' | b'V'))
        {
            let rest = &text[i..];
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(rest.len());
            let mut it = rest[..end].split('.').map(|p| p.parse::<u64>());
            if let (Some(Ok(a)), Some(Ok(b)), Some(Ok(c))) = (it.next(), it.next(), it.next()) {
                return Some((a, b, c));
            }
            i += end.max(1);
        } else {
            i += 1;
        }
    }
    None
}

/// `2026-09-02T10:30:15.123456+00:00` / `...Z` to Unix seconds.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let t = text.trim();
    let b = t.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let mut rest = &t[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let end = frac
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(frac.len());
        rest = &frac[end..];
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            if digits.len() < 2 {
                return None;
            }
            let oh: i64 = digits[..2].parse().ok()?;
            let om: i64 = digits.get(2..4).map_or(Some(0), |m| m.parse().ok())?;
            sign * (oh * 3600 + om * 60)
        }
    };
    // days from civil (Howard Hinnant)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s - offset)
}

/// At most 200 chars of `text`, for error messages.
pub fn snippet(text: &str) -> String {
    let t = text.trim();
    let mut s: String = t.chars().take(200).collect();
    if t.chars().count() > 200 {
        s.push_str("...");
    }
    s
}
