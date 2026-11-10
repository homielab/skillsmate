//! Checking for a newer SkillsMate release on GitHub.

use std::time::Duration;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::sanitize::sanitize_display;

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// `owner/repo` from a `https://github.com/owner/repo` URL.
pub fn github_repo(url: &str) -> Option<String> {
    let rest = url.trim_end_matches('/').trim_end_matches(".git");
    let rest = rest.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let valid = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (parts.next().is_none() && valid(owner) && valid(repo)).then(|| format!("{owner}/{repo}"))
}

/// The latest published release, or `None` if the repo has none yet.
pub async fn latest_release(repo: &str) -> Result<Option<Release>> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("skillsmate/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .https_only(true)
        .build()?;
    let resp = http
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let raw: RawRelease = resp.error_for_status()?.json().await?;
    if raw.draft || raw.prerelease {
        return Ok(None);
    }
    let expected = format!("https://github.com/{repo}/releases/");
    if !raw.html_url.starts_with(&expected) {
        return Err(Error::Parse("unexpected release URL".into()));
    }
    Ok(Some(Release {
        version: sanitize_display(raw.tag_name.trim_start_matches('v')),
        url: raw.html_url,
    }))
}

/// Whether `candidate` is a newer version than `current` (`1.10.0` > `1.9.2`;
/// a pre-release such as `2.0.0-beta` is older than `2.0.0`).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        let mut nums: Vec<u64> = core.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        nums.resize(3, 0);
        (nums, pre)
    }
    let (a, a_pre) = parse(candidate);
    let (b, b_pre) = parse(current);
    match a.cmp(&b) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => !a_pre && b_pre,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.10.0", "1.9.2"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("2.0.0", "2.0.0-beta.1"));
        assert!(!is_newer("2.0.0-beta.1", "2.0.0"));
    }

    #[test]
    fn repo_from_url() {
        assert_eq!(
            github_repo("https://github.com/acme/skillsmate").as_deref(),
            Some("acme/skillsmate")
        );
        assert_eq!(
            github_repo("https://github.com/acme/skillsmate.git/").as_deref(),
            Some("acme/skillsmate")
        );
        assert_eq!(github_repo("https://example.com/acme/x"), None);
        assert_eq!(github_repo("https://github.com/acme/x/y"), None);
    }
}
