//! Read-only data from the skills.sh directory: security audits, the official
//! (first-party) list, and topic collections.
//!
//! Sources, checked 2026-11-03:
//! - Audits: `GET https://skills.sh/api/v1/skills/audit/{source}/{slug}`. Works
//!   without a token; 404 means no partner has audited the skill yet.
//! - Official: the documented `/api/v1/skills/curated` endpoint needs a Vercel
//!   deployment token, so we read the same dataset from the data the
//!   `www.skills.sh/official` page embeds (`{"owners":[...]}` in its Next.js
//!   payload).
//! - Topics: the `/topic` index cards, and each `/topic/{slug}` page's
//!   schema.org `CollectionPage` JSON-LD.
//!
//! Page parsing is inherently fragile, so every parser fails soft with
//! [`Error::Parse`] and callers fall back to a link to the website. The site's
//! terms encourage reasonable use with caching; the app fetches these pages
//! only when the user opens the matching page and keeps them for the session.
//! All text is untrusted and passes through `sanitize_display`.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::files::{is_installable, validate_snapshot_path, FileMap, MAX_FILES, MAX_TOTAL_BYTES};
use crate::sanitize::sanitize_display;

pub const API_BASE: &str = "https://skills.sh";
pub const SITE_BASE: &str = "https://www.skills.sh";
/// The official page is ~0.5 MB; anything far larger is not what we expect.
const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone)]
pub struct DirectoryClient {
    http: reqwest::Client,
}

impl DirectoryClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("skillsmate/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .https_only(true)
            .build()?;
        Ok(Self { http })
    }

    /// Audit results for one skill, or `None` if it hasn't been audited yet.
    pub async fn audit(&self, source: &str, slug: &str) -> Result<Option<SkillAudit>> {
        if !is_safe_segment_path(source) || !is_safe_segment_path(slug) {
            return Err(Error::Parse(format!("invalid skill id {source}/{slug}")));
        }
        let url = format!("{API_BASE}/api/v1/skills/audit/{source}/{slug}");
        let resp = self.http.get(url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = read_capped(resp.error_for_status()?, MAX_JSON_BYTES).await?;
        parse_audit(&body).map(Some)
    }

    pub async fn official(&self) -> Result<Official> {
        let html = self.page("/official").await?;
        parse_official(&html)
    }

    pub async fn topics(&self) -> Result<Vec<Topic>> {
        let html = self.page("/topic").await?;
        parse_topic_index(&html)
    }

    /// The published files of a skill, from the same skills.sh download API
    /// the CLI uses (`/api/download/{owner}/{repo}/{slug}`). `None` when
    /// skills.sh has no snapshot. Only GitHub `owner/repo` sources are served.
    pub async fn download(&self, source: &str, slug: &str) -> Result<Option<FileMap>> {
        let mut parts = source.split('/');
        let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
            return Ok(None);
        };
        if !is_safe_segment_path(source) || !is_safe_segment_path(slug) || slug.contains('/') {
            return Err(Error::Parse(format!("invalid skill id {source}/{slug}")));
        }
        let url = format!("{API_BASE}/api/download/{owner}/{repo}/{slug}");
        let resp = self.http.get(url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        // JSON escaping can roughly double the size of the files.
        let body = read_capped(resp.error_for_status()?, 2 * MAX_TOTAL_BYTES as usize).await?;
        parse_snapshot(&body).map(Some)
    }

    pub async fn topic(&self, slug: &str) -> Result<TopicDetail> {
        if slug.is_empty() || !slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(Error::Parse(format!("invalid topic slug {slug}")));
        }
        let html = self.page(&format!("/topic/{slug}")).await?;
        parse_topic(&html)
    }

    async fn page(&self, path: &str) -> Result<String> {
        let resp = self.http.get(format!("{SITE_BASE}{path}")).send().await?;
        read_capped(resp.error_for_status()?, MAX_PAGE_BYTES).await
    }
}

async fn read_capped(mut resp: reqwest::Response, cap: usize) -> Result<String> {
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() + chunk.len() > cap {
            return Err(Error::Parse(format!("response larger than {cap} bytes")));
        }
        buf.extend_from_slice(&chunk);
    }
    String::from_utf8(buf).map_err(|_| Error::Parse("response is not UTF-8".into()))
}

/// `owner/repo` or a domain: no empty, `.` or `..` segments, URL-safe bytes.
fn is_safe_segment_path(s: &str) -> bool {
    !s.is_empty()
        && s.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
}

// ---------------------------------------------------------------- audits ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditStatus {
    Pass,
    Warn,
    Fail,
    Unknown,
}

impl AuditStatus {
    fn parse(s: &str) -> Self {
        match s {
            "pass" => Self::Pass,
            "warn" => Self::Warn,
            "fail" => Self::Fail,
            _ => Self::Unknown,
        }
    }

    /// Severity for picking the worst result: fail > warn > unknown > pass.
    fn severity(self) -> u8 {
        match self {
            Self::Pass => 0,
            Self::Unknown => 1,
            Self::Warn => 2,
            Self::Fail => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "Pass",
            Self::Warn => "Warning",
            Self::Fail => "Fail",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub provider: String,
    pub status: AuditStatus,
    pub summary: String,
    pub audited_at: String,
    /// `NONE`, `SAFE`, `LOW`, `MEDIUM`, `HIGH`, `CRITICAL` where provided.
    pub risk_level: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillAudit {
    pub entries: Vec<AuditEntry>,
}

impl SkillAudit {
    /// The worst status across providers.
    pub fn overall(&self) -> AuditStatus {
        self.entries
            .iter()
            .map(|e| e.status)
            .max_by_key(|s| s.severity())
            .unwrap_or(AuditStatus::Unknown)
    }

    /// The entries that did not pass, worst first.
    pub fn findings(&self) -> Vec<&AuditEntry> {
        let mut out: Vec<&AuditEntry> = self
            .entries
            .iter()
            .filter(|e| e.status != AuditStatus::Pass)
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.status.severity()));
        out
    }
}

#[derive(Deserialize)]
struct RawAudit {
    #[serde(default)]
    audits: Vec<RawAuditEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAuditEntry {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    audited_at: String,
    #[serde(default)]
    risk_level: Option<String>,
}

pub fn parse_audit(json: &str) -> Result<SkillAudit> {
    let raw: RawAudit =
        serde_json::from_str(json).map_err(|e| Error::Parse(format!("audit response: {e}")))?;
    Ok(SkillAudit {
        entries: raw
            .audits
            .into_iter()
            .map(|a| AuditEntry {
                provider: sanitize_display(&a.provider),
                status: AuditStatus::parse(&a.status),
                summary: sanitize_display(&a.summary),
                audited_at: sanitize_display(&a.audited_at),
                risk_level: a.risk_level.map(|r| sanitize_display(&r)),
            })
            .collect(),
    })
}

// ------------------------------------------------------------- downloads ---

#[derive(Deserialize)]
struct RawSnapshot {
    files: Vec<RawSnapshotFile>,
}

#[derive(Deserialize)]
struct RawSnapshotFile {
    path: String,
    contents: String,
}

/// Validates a download before anything touches disk: every path must be a
/// plain relative path, limits on count and size apply, and SKILL.md must be
/// present. Excluded files (metadata.json, .git, caches) are dropped.
pub fn parse_snapshot(json: &str) -> Result<FileMap> {
    let raw: RawSnapshot =
        serde_json::from_str(json).map_err(|e| Error::Parse(format!("download: {e}")))?;
    let mut files = FileMap::new();
    let mut total = 0u64;
    for f in raw.files {
        validate_snapshot_path(&f.path)?;
        if !is_installable(&f.path) {
            continue;
        }
        total += f.contents.len() as u64;
        if files.len() >= MAX_FILES || total > MAX_TOTAL_BYTES {
            return Err(Error::Parse("download is larger than the limit".into()));
        }
        if files
            .insert(f.path.clone(), f.contents.into_bytes())
            .is_some()
        {
            return Err(Error::Parse(format!("duplicate file {}", f.path)));
        }
    }
    if !files.contains_key("SKILL.md") {
        return Err(Error::Parse("download has no SKILL.md".into()));
    }
    Ok(files)
}

/// The slug skills.sh derives from a skill name (the CLI's `toSkillSlug`).
pub fn to_skill_slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        let c = if c.is_whitespace() || c == '_' {
            '-'
        } else {
            c
        };
        let allowed = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        if allowed && !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    out.trim_matches('-').to_owned()
}

// -------------------------------------------------------------- official ---

#[derive(Debug, Clone, PartialEq)]
pub struct Official {
    pub owners: Vec<OfficialOwner>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfficialOwner {
    pub owner: String,
    pub total_installs: u64,
    pub repos: Vec<OfficialRepo>,
}

impl OfficialOwner {
    pub fn skill_count(&self) -> usize {
        self.repos.iter().map(|r| r.skills.len()).sum()
    }

    /// Skill names across repos, most installed first.
    pub fn top_skills(&self, n: usize) -> Vec<&str> {
        let mut all: Vec<&OfficialSkill> = self.repos.iter().flat_map(|r| &r.skills).collect();
        all.sort_by(|a, b| {
            b.installs
                .cmp(&a.installs)
                .then_with(|| a.name.cmp(&b.name))
        });
        all.into_iter().take(n).map(|s| s.name.as_str()).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfficialRepo {
    pub repo: String,
    pub total_installs: u64,
    pub skills: Vec<OfficialSkill>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfficialSkill {
    pub name: String,
    pub installs: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOfficial {
    owners: Vec<RawOwner>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOwner {
    owner: String,
    #[serde(default)]
    total_installs: u64,
    #[serde(default)]
    repos: Vec<RawRepo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepo {
    repo: String,
    #[serde(default)]
    total_installs: u64,
    #[serde(default)]
    skills: Vec<RawOfficialSkill>,
}

#[derive(Deserialize)]
struct RawOfficialSkill {
    name: String,
    #[serde(default)]
    installs: u64,
}

pub fn parse_official(html: &str) -> Result<Official> {
    let payload = next_flight_payload(html);
    let start = payload
        .find(r#"{"owners":["#)
        .ok_or_else(|| Error::Parse("official list not found in page".into()))?;
    let raw: RawOfficial = first_json_value(&payload[start..])
        .ok_or_else(|| Error::Parse("official list is not valid JSON".into()))?;
    let mut owners: Vec<OfficialOwner> = raw
        .owners
        .into_iter()
        .map(|o| OfficialOwner {
            owner: sanitize_display(&o.owner),
            total_installs: o.total_installs,
            repos: o
                .repos
                .into_iter()
                .map(|r| OfficialRepo {
                    repo: sanitize_display(&r.repo),
                    total_installs: r.total_installs,
                    skills: r
                        .skills
                        .into_iter()
                        .map(|s| OfficialSkill {
                            name: sanitize_display(&s.name),
                            installs: s.installs,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    if owners.is_empty() {
        return Err(Error::Parse("official list is empty".into()));
    }
    owners.sort_by(|a, b| {
        b.total_installs
            .cmp(&a.total_installs)
            .then_with(|| a.owner.cmp(&b.owner))
    });
    Ok(Official { owners })
}

/// Concatenates the string chunks a Next.js page streams through
/// `self.__next_f.push([1,"..."])` script tags.
fn next_flight_payload(html: &str) -> String {
    const MARKER: &str = "self.__next_f.push([1,";
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find(MARKER) {
        rest = &rest[i + MARKER.len()..];
        if let Some(chunk) = first_json_value::<String>(rest) {
            out.push_str(&chunk);
        }
    }
    out
}

/// Parses the JSON value at the start of `s`, ignoring whatever follows it.
fn first_json_value<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::Deserializer::from_str(s)
        .into_iter::<T>()
        .next()?
        .ok()
}

// ---------------------------------------------------------------- topics ---

#[derive(Debug, Clone, PartialEq)]
pub struct Topic {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub skill_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TopicDetail {
    pub title: String,
    pub description: String,
    pub skills: Vec<TopicSkill>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TopicSkill {
    pub name: String,
    pub description: String,
    /// `owner/repo` (or a domain), from the skill's skills.sh URL.
    pub source: String,
    /// Path on skills.sh, e.g. `anthropics/skills/frontend-design`.
    pub id: String,
}

impl TopicSkill {
    pub fn url(&self) -> String {
        format!("{SITE_BASE}/{}", self.id)
    }
}

/// Topic cards on `/topic`: `<a href="/topic/{slug}">` wrapping a title, a
/// description, and "{n} skills". Navigation links with only a title are
/// skipped; the first card per slug wins.
pub fn parse_topic_index(html: &str) -> Result<Vec<Topic>> {
    const HREF: &str = "href=\"/topic/";
    let mut topics: Vec<Topic> = Vec::new();
    // Navigation links carry the short topic name ("Agent workflows"); cards
    // carry "<name> skills" headings that don't always pluralise the same way.
    let mut nav_names: Vec<(String, String)> = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(HREF) {
        rest = &rest[i + HREF.len()..];
        let Some(end) = rest.find('"') else { break };
        let slug = &rest[..end];
        let (Some(open_end), Some(close)) = (rest.find('>'), rest.find("</a>")) else {
            break;
        };
        if close < open_end {
            continue;
        }
        let parts = text_parts(&rest[open_end + 1..close]);
        rest = &rest[close..];
        let valid_slug =
            !slug.is_empty() && slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if valid_slug && parts.len() == 1 {
            nav_names.push((slug.to_owned(), parts[0].clone()));
        }
        if !valid_slug || parts.len() < 2 || topics.iter().any(|t| t.slug == slug) {
            continue;
        }
        let title = parts[0].trim_end_matches(" skills").to_owned();
        let skill_count = parts.iter().skip(2).find_map(|p| p.parse::<u32>().ok());
        topics.push(Topic {
            slug: slug.to_owned(),
            title: sanitize_display(&title),
            description: sanitize_display(&parts[1]),
            skill_count,
        });
    }
    if topics.is_empty() {
        return Err(Error::Parse("no topics found in page".into()));
    }
    for topic in &mut topics {
        if let Some((_, name)) = nav_names.iter().find(|(s, _)| *s == topic.slug) {
            topic.title = sanitize_display(name);
        }
    }
    Ok(topics)
}

/// A topic page's schema.org `CollectionPage`: name, description, and
/// `hasPart` entries with name, description and URL.
pub fn parse_topic(html: &str) -> Result<TopicDetail> {
    for json in json_ld_blocks(html) {
        let Ok(v) = serde_json::from_str::<Value>(json) else {
            continue;
        };
        if v.get("@type").and_then(Value::as_str) != Some("CollectionPage") {
            continue;
        }
        let text =
            |v: &Value, k: &str| sanitize_display(v.get(k).and_then(Value::as_str).unwrap_or(""));
        let skills = v
            .get("hasPart")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| {
                        let id = skill_id_from_url(p.get("url")?.as_str()?)?;
                        let (source, _) = id.rsplit_once('/')?;
                        Some(TopicSkill {
                            name: text(p, "name"),
                            description: text(p, "description"),
                            source: source.to_owned(),
                            id: id.clone(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Ok(TopicDetail {
            title: text(&v, "name").trim_end_matches(" skills").to_owned(),
            description: text(&v, "description"),
            skills,
        });
    }
    Err(Error::Parse("topic data not found in page".into()))
}

/// `https://www.skills.sh/owner/repo/skill` → `owner/repo/skill`.
fn skill_id_from_url(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("https://www.skills.sh/")
        .or_else(|| url.strip_prefix("https://skills.sh/"))?;
    let path = path.trim_end_matches('/');
    (path.contains('/') && is_safe_segment_path(path)).then(|| path.to_owned())
}

fn json_ld_blocks(html: &str) -> Vec<&str> {
    const OPEN: &str = "<script type=\"application/ld+json\"";
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(OPEN) {
        rest = &rest[i + OPEN.len()..];
        let Some(gt) = rest.find('>') else { break };
        let Some(end) = rest.find("</script>") else {
            break;
        };
        if end > gt {
            out.push(&rest[gt + 1..end]);
        }
        rest = &rest[end..];
    }
    out
}

/// The non-empty text runs between tags, with entities decoded.
fn text_parts(fragment: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => {
                in_tag = true;
                push_part(&mut parts, &mut current);
            }
            '>' => in_tag = false,
            _ if !in_tag => current.push(c),
            _ => {}
        }
    }
    push_part(&mut parts, &mut current);
    parts
}

fn push_part(parts: &mut Vec<String>, current: &mut String) {
    let text = decode_entities(current.trim());
    if !text.is_empty() {
        parts.push(text);
    }
    current.clear();
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from the live responses/pages on 2026-11-03.
    const AUDIT: &str = r#"{"id":"vercel-labs/agent-skills/web-design-guidelines","source":"vercel-labs/agent-skills","slug":"web-design-guidelines","audits":[
        {"provider":"Gen Agent Trust Hub","slug":"agent-trust-hub","status":"pass","summary":"This skill is safe.","auditedAt":"2026-09-15T09:11:58.047Z","riskLevel":"SAFE","categories":["EXTERNAL_DOWNLOADS"]},
        {"provider":"Socket","slug":"socket","status":"warn","summary":"1 alert: gptAnomaly","auditedAt":"2026-03-18T16:51:36.290Z"},
        {"provider":"Snyk","slug":"snyk","status":"pass","summary":"Risk: LOW · No issues","auditedAt":"2026-09-15T09:11:42.193108+00:00","riskLevel":"LOW"}]}"#;

    const OFFICIAL_HTML: &str = r#"<html><script>self.__next_f.push([1,"0:[\"$\",\"$L1\"]\n"])</script><script>self.__next_f.push([1,"28:[\"$\",\"$L32\",null,{\"data\":{\"owners\":[{\"owner\":\"aave\",\"repos\":[{\"repo\":\"aave/skills\",\"totalInstalls\":10,\"skills\":[{\"name\":\"deleverage\",\"installs\":2}]}],\"totalInstalls\":10,\"featuredRepo\":\"aave/skills\",\"featuredSkill\":\"deleverage\"},{\"owner\":\"anthropics\",\"repos\":[{\"repo\":\"anthropics/skills\",\"totalInstalls\":1526646,\"skills\":[{\"name\":\"pdf\",\"installs\":109704},{\"name\":\"frontend-design\",\"installs\":421750}]}],\"totalInstalls\":1526646}],\"totalOwners\":2,\"totalRepos\":2,\"totalSkills\":3,\"generatedAt\":\"2026-11-03T00:00:00Z\"}}]\n"])</script></html>"#;

    const TOPIC_INDEX_HTML: &str = r#"<nav><a href="/topic/react" class="x">React</a></nav>
        <a class="card" href="/topic/react"><h2>Frontend &amp; React skills</h2><p>Performance rules for production-quality React.</p><span>6</span> <span>skills</span></a>
        <a class="card" href="/topic/design"><h2>Design &amp; UI skills</h2><p>Taste and frameworks &#x27;polished&#x27;.</p><span>16</span> <span>skills</span></a>
        <a href="/topic/../evil"><h2>Bad</h2><p>x</p></a>
        <a href="/topic/design">Design &amp; UI</a>"#;

    const TOPIC_HTML: &str = r#"<script type="application/ld+json">{"@context":"https://schema.org","@type":"BreadcrumbList"}</script>
        <script type="application/ld+json">{"@context":"https://schema.org","@type":"CollectionPage","name":"Design & UI skills","description":"Design skills give your agent taste.","url":"https://www.skills.sh/topic/design","hasPart":[
          {"@type":"SoftwareApplication","name":"frontend-design","description":"Frontend design patterns","url":"https://www.skills.sh/anthropics/skills/frontend-design"},
          {"@type":"SoftwareApplication","name":"ui-taste","description":"Two-segment id","url":"https://www.skills.sh/uizze.sh/ui-taste"},
          {"@type":"SoftwareApplication","name":"evil","description":"x","url":"https://evil.example/a/b"}]}</script>"#;

    #[test]
    fn audit_parses_and_ranks_findings() {
        let audit = parse_audit(AUDIT).expect("valid");
        assert_eq!(audit.entries.len(), 3);
        assert_eq!(audit.overall(), AuditStatus::Warn);
        let findings = audit.findings();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].provider, "Socket");
        assert_eq!(audit.entries[2].summary, "Risk: LOW · No issues");
    }

    #[test]
    fn empty_audit_is_unknown() {
        let audit = parse_audit(r#"{"audits":[]}"#).expect("valid");
        assert_eq!(audit.overall(), AuditStatus::Unknown);
    }

    #[test]
    fn official_extracted_from_flight_payload() {
        let official = parse_official(OFFICIAL_HTML).expect("valid");
        assert_eq!(official.owners.len(), 2);
        // Sorted by total installs.
        assert_eq!(official.owners[0].owner, "anthropics");
        assert_eq!(official.owners[0].skill_count(), 2);
        assert_eq!(official.owners[0].top_skills(1), vec!["frontend-design"]);
    }

    #[test]
    fn official_missing_is_parse_error() {
        assert!(matches!(
            parse_official("<html></html>"),
            Err(Error::Parse(_))
        ));
    }

    #[test]
    fn topic_index_reads_cards_only() {
        let topics = parse_topic_index(TOPIC_INDEX_HTML).expect("valid");
        assert_eq!(topics.len(), 2);
        assert_eq!(topics[0].slug, "react");
        // Card heading when there's no nav link; the nav name when there is.
        assert_eq!(topics[0].title, "React");
        assert_eq!(topics[1].title, "Design & UI");
        assert_eq!(topics[0].skill_count, Some(6));
        assert_eq!(topics[1].description, "Taste and frameworks 'polished'.");
    }

    #[test]
    fn topic_page_reads_json_ld_and_rejects_foreign_urls() {
        let topic = parse_topic(TOPIC_HTML).expect("valid");
        assert_eq!(topic.title, "Design & UI");
        assert_eq!(topic.skills.len(), 2);
        assert_eq!(topic.skills[0].source, "anthropics/skills");
        assert_eq!(topic.skills[1].source, "uizze.sh");
        assert_eq!(
            topic.skills[0].url(),
            "https://www.skills.sh/anthropics/skills/frontend-design"
        );
    }

    #[test]
    fn snapshot_is_validated() {
        let ok = r#"{"files":[{"path":"SKILL.md","contents":"---\nname: a\n---"},{"path":"metadata.json","contents":"{}"},{"path":"references/x.md","contents":"x"}],"hash":"h"}"#;
        let files = parse_snapshot(ok).expect("valid");
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            vec!["SKILL.md", "references/x.md"]
        );
        let evil = r#"{"files":[{"path":"SKILL.md","contents":"x"},{"path":"../../.bashrc","contents":"x"}]}"#;
        assert!(parse_snapshot(evil).is_err());
        let no_skill = r#"{"files":[{"path":"README.md","contents":"x"}]}"#;
        assert!(parse_snapshot(no_skill).is_err());
    }

    #[test]
    fn slugs_match_cli() {
        assert_eq!(
            to_skill_slug("Convex Best Practices"),
            "convex-best-practices"
        );
        assert_eq!(to_skill_slug("my_skill--v2!"), "my-skill-v2");
        assert_eq!(to_skill_slug("frontend-design"), "frontend-design");
    }

    #[test]
    fn segment_paths_are_validated() {
        assert!(is_safe_segment_path("vercel-labs/agent-skills"));
        assert!(is_safe_segment_path("open.feishu.cn"));
        assert!(!is_safe_segment_path("a/../b"));
        assert!(!is_safe_segment_path("a//b"));
        assert!(!is_safe_segment_path("a/b?x=1"));
    }
}
