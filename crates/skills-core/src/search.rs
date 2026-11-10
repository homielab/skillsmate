//! Client for the skills.sh directory API (the same endpoint `skills find` uses).
//!
//! All text from the API is treated as untrusted and sanitized before it
//! leaves this module. We deliberately do not send the CLI's telemetry.

use std::time::Duration;

use serde::Deserialize;

use crate::error::Result;
use crate::sanitize::sanitize_display;

pub const DEFAULT_API_BASE: &str = "https://skills.sh";
/// The CLI rejects shorter queries; so does the API.
pub const MIN_QUERY_LEN: usize = 2;
const RESULT_LIMIT: u32 = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub name: String,
    /// skills.sh slug; the page is `https://skills.sh/<slug>`.
    pub slug: String,
    /// Install source, e.g. `owner/repo`.
    pub source: String,
    pub installs: u64,
}

impl SearchHit {
    pub fn page_url(&self, api_base: &str) -> String {
        format!("{}/{}", api_base.trim_end_matches('/'), self.slug)
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    skills: Vec<ApiSkill>,
}

#[derive(Deserialize)]
struct ApiSkill {
    id: String,
    name: String,
    #[serde(default)]
    installs: u64,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchClient {
    http: reqwest::Client,
    base: String,
}

impl SearchClient {
    pub fn new() -> Result<Self> {
        Self::with_base(DEFAULT_API_BASE)
    }

    pub fn with_base(base: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("skillsmate/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .https_only(true)
            .build()?;
        Ok(Self {
            http,
            base: base.into().trim_end_matches('/').to_owned(),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Searches the directory. Results are sorted by install count, highest first.
    /// Queries shorter than [`MIN_QUERY_LEN`] return no results.
    pub async fn search(&self, query: &str, owner: Option<&str>) -> Result<Vec<SearchHit>> {
        let query = query.trim();
        if query.chars().count() < MIN_QUERY_LEN {
            return Ok(Vec::new());
        }
        let limit = RESULT_LIMIT.to_string();
        let mut params = vec![("q", query), ("limit", limit.as_str())];
        if let Some(owner) = owner {
            params.push(("owner", owner));
        }
        let resp: ApiResponse = self
            .http
            .get(format!("{}/api/search", self.base))
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(into_hits(resp))
    }
}

fn into_hits(resp: ApiResponse) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = resp
        .skills
        .into_iter()
        .map(|s| SearchHit {
            name: sanitize_display(&s.name),
            slug: sanitize_display(&s.id),
            source: sanitize_display(s.source.as_deref().unwrap_or_default()),
            installs: s.installs,
        })
        .collect();
    hits.sort_by_key(|h| std::cmp::Reverse(h.installs));
    hits
}

/// Formats an install count like the CLI: `1.2K installs`.
pub fn format_installs(count: u64) -> String {
    fn compact(value: f64, suffix: &str) -> String {
        let s = format!("{value:.1}");
        let s = s.strip_suffix(".0").unwrap_or(&s);
        format!("{s}{suffix} installs")
    }
    match count {
        0 => String::new(),
        1 => "1 install".into(),
        2..=999 => format!("{count} installs"),
        1_000..=999_999 => compact(count as f64 / 1_000.0, "K"),
        _ => compact(count as f64 / 1_000_000.0, "M"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sorts_api_response() {
        let resp: ApiResponse = serde_json::from_str(
            r#"{"skills":[
                {"id":"o/r/a","name":"a","installs":5,"source":"o/r"},
                {"id":"o/r/b","name":"b\u001b[2J","installs":50}
            ]}"#,
        )
        .expect("valid json");
        let hits = into_hits(resp);
        assert_eq!(hits[0].name, "b[2J");
        assert_eq!(hits[0].source, "");
        assert_eq!(
            hits[1].page_url("https://skills.sh/"),
            "https://skills.sh/o/r/a"
        );
    }

    #[test]
    fn formats_like_cli() {
        assert_eq!(format_installs(0), "");
        assert_eq!(format_installs(1), "1 install");
        assert_eq!(format_installs(999), "999 installs");
        assert_eq!(format_installs(1_000), "1K installs");
        assert_eq!(format_installs(1_240), "1.2K installs");
        assert_eq!(format_installs(3_400_000), "3.4M installs");
    }

    #[tokio::test]
    async fn short_queries_skip_the_network() {
        let client = SearchClient::with_base("https://invalid.example").expect("client");
        assert!(client.search(" a ", None).await.expect("ok").is_empty());
    }
}
