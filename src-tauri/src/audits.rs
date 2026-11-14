//! Security audits for installed skills, from skills.sh's audit partners.
//!
//! Checking is an explicit user action because it sends the names and sources
//! of installed skills to skills.sh.

use std::sync::Arc;

use serde::Serialize;
use skills_core::directory::{AuditStatus, DirectoryClient, SkillAudit, SITE_BASE};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::library::{Attention, AuditTarget, Overview};
use crate::state::{plural, AppState};

/// Parallel requests to skills.sh while checking.
const CONCURRENCY: usize = 4;

enum Outcome {
    Local,
    NotAudited,
    Failed(String),
    Audited(SkillAudit),
}

#[derive(Serialize)]
struct AuditRow {
    name: String,
    source: String,
    /// Status mark kind.
    kind: u8,
    label: String,
    detail: String,
    url: String,
}

#[derive(Serialize)]
pub struct AuditsView {
    rows: Vec<AuditRow>,
    status: String,
    flagged: usize,
    /// The Overview, with the audit result in its "Needs attention" list.
    overview: Overview,
}

#[tauri::command]
pub async fn check_audits(state: tauri::State<'_, AppState>) -> Result<AuditsView, String> {
    let client = state.directory()?;
    let targets = state.lock().library.audit_targets.clone();
    let results = fetch_all(client, targets).await;

    let mut rows: Vec<(u8, AuditRow)> = results.iter().map(|(t, o)| row(t, o)).collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));

    let count = |k: u8| rows.iter().filter(|(_, r)| r.kind == k).count();
    let (passed, warned, failed) = (count(0), count(1), count(2));
    let local = results
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Local))
        .count();
    let unknown = rows.len() - passed - warned - failed - local;

    let mut parts = vec![format!("{passed} passed")];
    if warned > 0 {
        parts.push(plural(warned, "warning", "warnings"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if unknown > 0 {
        parts.push(format!("{unknown} not audited"));
    }
    if local > 0 {
        parts.push(format!("{local} local"));
    }

    // Surface the result on the Overview too.
    let mut st = state.lock();
    let attention = &mut st.library.overview.attention;
    attention.retain(|a| !a.text.starts_with("Security audit"));
    let flagged = warned + failed;
    if flagged > 0 {
        attention.insert(
            0,
            Attention {
                text: format!(
                    "Security audit: {} flagged by skills.sh auditors.",
                    plural(flagged, "skill", "skills")
                ),
                kind: if failed > 0 { 2 } else { 1 },
            },
        );
        attention.retain(|a| a.text != "No problems found.");
    } else if passed > 0 {
        attention.push(Attention {
            text: format!(
                "Security audit: {} passed.",
                plural(passed, "skill", "skills")
            ),
            kind: 0,
        });
    }

    Ok(AuditsView {
        rows: rows.into_iter().map(|(_, r)| r).collect(),
        status: parts.join(" · "),
        flagged,
        overview: st.library.overview.clone(),
    })
}

async fn fetch_all(
    client: DirectoryClient,
    targets: Vec<AuditTarget>,
) -> Vec<(AuditTarget, Outcome)> {
    let permits = Arc::new(Semaphore::new(CONCURRENCY));
    let mut set = JoinSet::new();
    for (i, target) in targets.into_iter().enumerate() {
        let client = client.clone();
        let permits = Arc::clone(&permits);
        set.spawn(async move {
            let outcome = match &target.source {
                None => Outcome::Local,
                Some(source) => {
                    let _permit = permits.acquire().await;
                    match client.audit(source, &target.slug).await {
                        Ok(Some(audit)) => Outcome::Audited(audit),
                        Ok(None) => Outcome::NotAudited,
                        Err(e) => Outcome::Failed(e.to_string()),
                    }
                }
            };
            (i, target, outcome)
        });
    }
    let mut out = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(result) = joined {
            out.push(result);
        }
    }
    out.sort_by_key(|(i, _, _)| *i);
    out.into_iter().map(|(_, t, o)| (t, o)).collect()
}

/// A row plus its sort rank: failures first, then warnings, unknowns, passes,
/// and local skills last.
fn row(t: &AuditTarget, outcome: &Outcome) -> (u8, AuditRow) {
    let url = t
        .source
        .as_ref()
        .map(|s| format!("{SITE_BASE}/{s}/{}", t.slug))
        .unwrap_or_default();
    let (rank, kind, label, detail) = match outcome {
        Outcome::Local => (
            4,
            3,
            "Local".to_owned(),
            "Not installed from skills.sh, so there's no public audit.".to_owned(),
        ),
        Outcome::NotAudited => (2, 3, "Not audited yet".to_owned(), String::new()),
        Outcome::Failed(e) => (2, 3, "Couldn't check".to_owned(), e.clone()),
        Outcome::Audited(audit) => {
            let findings: Vec<String> = audit
                .findings()
                .iter()
                .map(|f| format!("{}: {}", f.provider, f.summary))
                .collect();
            match audit.overall() {
                AuditStatus::Fail => (0, 2, "Failed".to_owned(), findings.join("\n")),
                AuditStatus::Warn => (1, 1, "Warning".to_owned(), findings.join("\n")),
                AuditStatus::Pass => (
                    3,
                    0,
                    "Passed".to_owned(),
                    format!(
                        "{}, no issues",
                        plural(audit.entries.len(), "auditor", "auditors")
                    ),
                ),
                AuditStatus::Unknown => (2, 3, "Unknown".to_owned(), findings.join("\n")),
            }
        }
    };
    (
        rank,
        AuditRow {
            name: t.name.clone(),
            source: t.source.clone().unwrap_or_else(|| "Local".into()),
            kind,
            label,
            detail,
            url,
        },
    )
}
