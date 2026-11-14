//! The Health page: local findings, online update checks, and one-click fixes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use skills_core::directory::{to_skill_slug, DirectoryClient};
use skills_core::files::{diff, read_skill_files, FileDiff, FileMap};
use skills_core::health::{round_k, Finding, Fix, Severity};
use skills_core::manage::{compare, load_baselines, UpdateState};
use skills_core::Env;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::library::{display_name, library_view, Attention, LibraryView};
use crate::state::{plural, AppState, State};

const CONCURRENCY: usize = 4;
pub const MAX_DIFF_LINES: usize = 400;

/// The latest update check for one skill.
pub struct UpdateInfo {
    pub state: UpdateState,
    pub diff: FileDiff,
    /// The published files, kept so "Update" doesn't download again.
    pub remote: FileMap,
    pub source: String,
}

impl UpdateInfo {
    /// Short label for the Installed list, "" when up to date.
    pub fn badge(&self) -> &'static str {
        match self.state {
            UpdateState::UpToDate => "",
            s => s.label(),
        }
    }

    /// Status mark kind.
    pub fn kind(&self) -> u8 {
        match self.state {
            UpdateState::UpToDate => 0,
            UpdateState::UpdateAvailable => 1,
            _ => 4,
        }
    }
}

#[derive(Default)]
pub struct UpdatesState {
    pub results: HashMap<String, UpdateInfo>,
    checked: usize,
    unavailable: usize,
}

struct Target {
    name: String,
    source: String,
    slugs: Vec<String>,
    path: PathBuf,
}

enum Checked {
    Done(UpdateInfo),
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingRow {
    title: String,
    detail: String,
    /// Display name of the skill it's about, "" for general findings.
    skill: String,
    /// Frontmatter name, used to open the skill.
    key: String,
    /// Status mark kind: 2 problem, 1 warning, 4 tip.
    kind: u8,
    /// Button label for a one-click fix, "" when there's none.
    fix_label: String,
    /// Position in the last scan's findings; identifies the finding to fix.
    index: usize,
}

impl FindingRow {
    pub fn new(index: usize, f: &Finding, skill: String) -> Self {
        Self {
            title: f.title.clone(),
            detail: f.detail.clone(),
            skill,
            key: f.skill.clone().unwrap_or_default(),
            kind: match f.severity {
                Severity::Problem => 2,
                Severity::Warning => 1,
                Severity::Tip => 4,
            },
            fix_label: match f.fix {
                Some(Fix::RemoveBrokenLink) => "Remove link",
                Some(Fix::RelinkCopy) => "Use main copy",
                None => "",
            }
            .to_owned(),
            index,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRow {
    name: String,
    key: String,
    source: String,
    label: String,
    kind: u8,
    summary: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthView {
    summary: String,
    problems: usize,
    warnings: usize,
    tips: usize,
    startup: String,
    startup_note: String,
    findings: Vec<FindingRow>,
    broken_links: usize,
    /// Number shown beside Health in the sidebar.
    badge: usize,
    updates: Vec<UpdateRow>,
    updates_status: String,
}

/// Builds the Health page and its nav badge from the latest scan and update
/// check.
pub fn health_view(st: &State) -> HealthView {
    let lib = &st.library;
    let h = &lib.health;
    let name_of = |key: &str| {
        lib.skills
            .iter()
            .find(|s| s.meta.name == key)
            .map_or_else(|| key.to_owned(), |s| s.meta.display_name())
    };

    let findings: Vec<FindingRow> = h
        .findings
        .iter()
        .enumerate()
        .map(|(i, f)| FindingRow::new(i, f, f.skill.as_deref().map(name_of).unwrap_or_default()))
        .collect();

    let (problems, warnings, tips) = (
        h.count(Severity::Problem),
        h.count(Severity::Warning),
        h.count(Severity::Tip),
    );
    let updates = &st.updates;
    let count = |state: UpdateState| {
        updates
            .results
            .values()
            .filter(|u| u.state == state)
            .count()
    };
    let available = count(UpdateState::UpdateAvailable);

    let mut parts = Vec::new();
    if problems > 0 {
        parts.push(plural(problems, "problem", "problems"));
    }
    if warnings > 0 {
        parts.push(plural(warnings, "warning", "warnings"));
    }
    if available > 0 {
        parts.push(plural(available, "update", "updates"));
    }
    if tips > 0 {
        parts.push(plural(tips, "tip", "tips"));
    }
    let summary = if parts.is_empty() {
        "Everything looks good".to_owned()
    } else {
        parts.join(" · ")
    };

    let (startup, startup_note) = match h.startup_by_agent.first() {
        Some((agent, tokens, n)) => (
            format!("~{} tokens", round_k(*tokens)),
            format!("{}, {}", display_name(agent), plural(*n, "skill", "skills")),
        ),
        None => ("0".into(), "No agent sees any skills".into()),
    };

    // Updates: anything not up to date, update-available first.
    let mut list: Vec<(&String, &UpdateInfo)> = updates
        .results
        .iter()
        .filter(|(_, u)| u.state != UpdateState::UpToDate)
        .collect();
    list.sort_by(|a, b| {
        (a.1.state != UpdateState::UpdateAvailable)
            .cmp(&(b.1.state != UpdateState::UpdateAvailable))
            .then_with(|| a.0.cmp(b.0))
    });
    let update_rows = list
        .into_iter()
        .map(|(key, u)| UpdateRow {
            name: name_of(key),
            key: key.clone(),
            source: u.source.clone(),
            label: u.state.label().to_owned(),
            kind: u.kind(),
            summary: u.diff.summary(),
        })
        .collect();

    // Empty until a check has run, so the "Check for updates" prompt stays;
    // after that, an empty list with a status line means everything is current.
    let updates_status = if updates.checked == 0 {
        String::new()
    } else {
        let mut s = format!("{} up to date", count(UpdateState::UpToDate));
        if available > 0 {
            s.push_str(&format!(" · {}", plural(available, "update", "updates")));
        }
        let edited = count(UpdateState::EditedLocally) + count(UpdateState::Diverged);
        if edited > 0 {
            s.push_str(&format!(" · {edited} edited locally"));
        }
        let differs = count(UpdateState::Differs);
        if differs > 0 {
            s.push_str(&format!(" · {differs} differ from skills.sh"));
        }
        if updates.unavailable > 0 {
            s.push_str(&format!(" · {} not on skills.sh", updates.unavailable));
        }
        s
    };

    HealthView {
        summary,
        problems,
        warnings,
        tips,
        startup,
        startup_note,
        broken_links: broken_links(&h.findings).len(),
        findings,
        badge: problems + warnings + available,
        updates: update_rows,
        updates_status,
    }
}

#[derive(Serialize)]
pub struct UpdatesChecked {
    library: LibraryView,
    /// Something to tell the user instead of results, e.g. nothing to check.
    note: Option<String>,
}

/// Compares every skills.sh skill with its published version. Sends the names
/// of those skills to skills.sh, so it only runs when the user asks.
#[tauri::command]
pub async fn check_updates(state: tauri::State<'_, AppState>) -> Result<UpdatesChecked, String> {
    let client = state.directory()?;
    let env = state.env()?;
    let targets: Vec<Target> = {
        let st = state.lock();
        let lib = &st.library;
        lib.skills
            .iter()
            .zip(&lib.facts)
            .filter_map(|(s, f)| {
                let source = f.source.clone()?;
                // Only GitHub owner/repo sources have downloadable snapshots.
                if source.matches('/').count() != 1 {
                    return None;
                }
                let mut slugs = vec![s.dir_name.clone()];
                let alt = to_skill_slug(&s.meta.name);
                if !slugs.contains(&alt) {
                    slugs.push(alt);
                }
                Some(Target {
                    name: s.meta.name.clone(),
                    source,
                    slugs,
                    path: s.primary_path().to_path_buf(),
                })
            })
            .collect()
    };
    if targets.is_empty() {
        return Ok(UpdatesChecked {
            library: library_view(&state.lock()),
            note: Some("No skills from skills.sh to check.".into()),
        });
    }

    let results = check_all(client, env, targets).await;

    let mut guard = state.lock();
    let st = &mut *guard;
    let u = &mut st.updates;
    u.checked = results.len();
    u.unavailable = 0;
    for (name, r) in results {
        match r {
            Checked::Done(info) => {
                u.results.insert(name, info);
            }
            Checked::Unavailable => {
                u.unavailable += 1;
                u.results.remove(&name);
            }
        }
    }
    let available = u
        .results
        .values()
        .filter(|i| i.state == UpdateState::UpdateAvailable)
        .count();
    // Surface the result on the Overview too.
    let attention = &mut st.library.overview.attention;
    attention
        .retain(|a| !a.text.contains("update available") && !a.text.contains("updates available"));
    if available > 0 {
        attention.insert(
            0,
            Attention {
                text: format!(
                    "{} available. See Health.",
                    plural(available, "update", "updates")
                ),
                kind: 1,
            },
        );
    }
    Ok(UpdatesChecked {
        library: library_view(st),
        note: None,
    })
}

async fn check_all(
    client: DirectoryClient,
    env: Arc<Env>,
    targets: Vec<Target>,
) -> Vec<(String, Checked)> {
    let baselines = Arc::new(
        tokio::task::spawn_blocking(move || load_baselines(&env))
            .await
            .unwrap_or_default(),
    );
    let permits = Arc::new(Semaphore::new(CONCURRENCY));
    let mut set = JoinSet::new();
    for t in targets {
        let client = client.clone();
        let permits = Arc::clone(&permits);
        let baselines = Arc::clone(&baselines);
        set.spawn(async move {
            let _permit = permits.acquire().await;
            let mut remote = None;
            for slug in &t.slugs {
                if let Ok(Some(files)) = client.download(&t.source, slug).await {
                    remote = Some(files);
                    break;
                }
            }
            let Some(remote) = remote else {
                return (t.name, Checked::Unavailable);
            };
            let path = t.path.clone();
            let local = tokio::task::spawn_blocking(move || read_skill_files(&path))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            let state = compare(&local, &remote, baselines.get(&t.name).map(String::as_str));
            let diff = diff(&local, &remote, MAX_DIFF_LINES);
            (
                t.name,
                Checked::Done(UpdateInfo {
                    state,
                    diff,
                    remote,
                    source: t.source,
                }),
            )
        });
    }
    let mut out = Vec::new();
    while let Some(Ok(r)) = set.join_next().await {
        out.push(r);
    }
    out
}

/// Paths of the broken links among `findings`, in finding order.
pub fn broken_links(findings: &[Finding]) -> Vec<PathBuf> {
    findings
        .iter()
        .filter(|f| f.fix == Some(Fix::RemoveBrokenLink))
        .filter_map(|f| f.path.clone())
        .collect()
}
