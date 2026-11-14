//! The skill detail view.

use serde::Serialize;
use skills_core::directory::{to_skill_slug, SITE_BASE};
use skills_core::files::{read_skill_files, DiffLineKind};
use skills_core::health::{pretty_path, round_k, SkillProfile};
use skills_core::markdown::{blocks, BlockKind};
use skills_core::scan::{InstalledSkill, LinkKind};
use skills_core::stats::{format_bytes, relative_date, today, SkillFacts};

use crate::health::FindingRow;
use crate::library::display_name;
use crate::state::{blocking, plural, AppState};

/// Everything the detail view needs from the state, gathered under the lock
/// and turned into a [`Detail`] after reading files off-thread.
struct Input {
    home: std::path::PathBuf,
    skill: InstalledSkill,
    facts: SkillFacts,
    profile: SkillProfile,
    findings: Vec<FindingRow>,
    update: Option<UpdateView>,
}

/// The update check result as the detail view shows it.
struct UpdateView {
    label: String,
    kind: u8,
    summary: String,
    diff: Vec<DiffRow>,
}

#[derive(Serialize)]
struct AgentUse {
    name: String,
    how: String,
}

#[derive(Serialize)]
struct DiffRow {
    /// 0 file header, 1 added, 2 removed, 3 context, 4 note
    kind: u8,
    text: String,
}

#[derive(Serialize)]
struct MdBlock {
    /// 0 h1, 1 h2, 2 h3, 3 paragraph, 4 bullet, 5 code, 6 quote
    kind: u8,
    text: String,
    depth: usize,
}

#[derive(Serialize)]
struct FileRow {
    path: String,
    size: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    key: String,
    name: String,
    description: String,
    version: String,
    source: String,
    managed: bool,
    location: String,
    size: String,
    files_count: String,
    active_tokens: String,
    startup_tokens: String,
    update_label: String,
    update_kind: u8,
    update_summary: String,
    can_update: bool,
    url: String,
    license: String,
    compatibility: String,
    allowed_tools: String,
    agents: Vec<AgentUse>,
    findings: Vec<FindingRow>,
    diff: Vec<DiffRow>,
    body: Vec<MdBlock>,
    files: Vec<FileRow>,
}

/// The detail view of one installed skill, or `None` if it no longer exists.
#[tauri::command]
pub async fn detail(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<Option<Detail>, String> {
    let env = state.env()?;
    let input = {
        let st = state.lock();
        let lib = &st.library;
        let Some(i) = lib.skills.iter().position(|s| s.meta.name == key) else {
            return Ok(None);
        };
        let findings = lib
            .health
            .findings
            .iter()
            .enumerate()
            .filter(|(_, f)| f.skill.as_deref() == Some(key.as_str()))
            .map(|(i, f)| FindingRow::new(i, f, String::new()))
            .collect();
        let update = st.updates.results.get(&key).map(|u| UpdateView {
            label: u.state.label().to_owned(),
            kind: u.kind(),
            summary: if u.diff.is_empty() {
                String::new()
            } else {
                u.diff.summary()
            },
            diff: u
                .diff
                .lines
                .iter()
                .map(|l| DiffRow {
                    kind: match l.kind {
                        DiffLineKind::File => 0,
                        DiffLineKind::Added => 1,
                        DiffLineKind::Removed => 2,
                        DiffLineKind::Context => 3,
                        DiffLineKind::Note => 4,
                    },
                    text: l.text.clone(),
                })
                .collect(),
        });
        Input {
            home: env.home().to_path_buf(),
            skill: lib.skills[i].clone(),
            facts: lib.facts.get(i).cloned().unwrap_or_default(),
            profile: lib.health.profiles.get(i).cloned().unwrap_or_default(),
            findings,
            update,
        }
    };
    blocking(move || build(input)).await.map(Some)
}

fn build(input: Input) -> Detail {
    let Input {
        home,
        skill,
        facts,
        profile,
        findings,
        update,
    } = input;
    let files = read_skill_files(skill.primary_path()).unwrap_or_default();

    let mut agents: Vec<(&str, &str)> = skill
        .installs
        .iter()
        .filter(|i| i.agent != "universal")
        .map(|i| {
            let how = match i.kind {
                LinkKind::Canonical => "reads the main copy",
                LinkKind::Symlink => "linked to the main copy",
                LinkKind::Copy => "has its own copy",
            };
            (display_name(i.agent), how)
        })
        .collect();
    agents.sort_unstable();
    agents.dedup();

    let today = today();
    let (update_label, update_kind, update_summary, diff, can_update) = match update {
        Some(u) => {
            let can = u.label != "Up to date";
            (u.label, u.kind, u.summary, u.diff, can)
        }
        None => {
            let note = match facts
                .updated_at
                .as_deref()
                .and_then(|d| relative_date(d, today))
            {
                Some(r) => format!("Updated {r}"),
                None if facts.source.is_some() => "Not checked yet".into(),
                None => "Installed by hand".into(),
            };
            let label = if facts.source.is_some() {
                "Unchecked"
            } else {
                "Local"
            };
            (label.to_owned(), 3, note, Vec::new(), false)
        }
    };

    let url = facts
        .source
        .as_ref()
        .filter(|s| s.matches('/').count() == 1)
        .map(|s| format!("{SITE_BASE}/{s}/{}", to_skill_slug(&skill.meta.name)))
        .unwrap_or_default();

    let body = blocks(&skill.meta.body)
        .into_iter()
        .map(|b| MdBlock {
            kind: match b.kind {
                BlockKind::Heading1 => 0,
                BlockKind::Heading2 => 1,
                BlockKind::Heading3 => 2,
                BlockKind::Paragraph => 3,
                BlockKind::Bullet => 4,
                BlockKind::Code => 5,
                BlockKind::Quote => 6,
            },
            text: b.text,
            depth: b.depth,
        })
        .collect();

    let file_rows = files
        .iter()
        .map(|(p, bytes)| FileRow {
            path: p.clone(),
            size: format_bytes(bytes.len() as u64),
        })
        .collect();

    Detail {
        key: skill.meta.name.clone(),
        name: skill.meta.display_name(),
        description: skill.meta.description.clone(),
        version: skill.meta.version.clone().unwrap_or_default(),
        source: facts.source.clone().unwrap_or_else(|| "Local".into()),
        managed: facts.source.is_some(),
        location: pretty_path(skill.primary_path(), &home),
        size: format_bytes(facts.size_bytes),
        files_count: plural(profile.files.max(files.len()), "file", "files"),
        active_tokens: format!("~{}", round_k(profile.body_tokens)),
        startup_tokens: format!("~{}", round_k(profile.startup_tokens)),
        update_label,
        update_kind,
        update_summary,
        can_update,
        url,
        license: skill.meta.license.clone().unwrap_or_default(),
        compatibility: skill.meta.compatibility.clone().unwrap_or_default(),
        allowed_tools: skill.meta.allowed_tools.clone().unwrap_or_default(),
        agents: agents
            .into_iter()
            .map(|(name, how)| AgentUse {
                name: name.to_owned(),
                how: how.to_owned(),
            })
            .collect(),
        findings,
        diff,
        body,
        files: file_rows,
    }
}
