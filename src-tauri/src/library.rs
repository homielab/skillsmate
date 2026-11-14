//! Local library: scanning installed skills and building the Overview,
//! Installed and Agents pages.

use std::collections::HashSet;

use serde::Serialize;
use skills_core::agents::{detected_agents, Agent};
use skills_core::health::{self, HealthReport, Severity};
use skills_core::lock::{read_global, LockState};
use skills_core::scan::{scan_scope, InstalledSkill};
use skills_core::stats::{
    format_bytes, relative_date, skill_facts, summarize, today, SkillFacts, Summary,
};
use skills_core::{Env, Scope};

use crate::health::{health_view, HealthView};
use crate::state::{blocking, plural, AppState, State};

const MAX_AGENT_BARS: usize = 8;
const MAX_SOURCE_BARS: usize = 6;

/// What the app keeps from the last scan.
#[derive(Default)]
pub struct LibraryState {
    /// Rows of the Installed page, without their update badge.
    rows: Vec<SkillRow>,
    /// The scanned skills, their facts and health, in the same order.
    pub skills: Vec<InstalledSkill>,
    pub facts: Vec<SkillFacts>,
    pub health: HealthReport,
    pub overview: Overview,
    /// Installed skills to check for audits.
    pub audit_targets: Vec<AuditTarget>,
    /// `(source, name)` and `(source, dir name)` of managed skills, for
    /// "Installed" markers on directory pages.
    pub installed: HashSet<(String, String)>,
    agents: Vec<AgentRow>,
    detected: usize,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRow {
    /// Frontmatter name; identifies the skill in every command.
    key: String,
    name: String,
    description: String,
    agents: String,
    source: String,
    meta: String,
    /// "Update available" etc., "" when unknown or up to date.
    update: String,
    /// Installed from skills.sh (has a source in the lockfile).
    managed: bool,
    /// At least one detected agent can see it.
    visible: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRow {
    name: String,
    id: String,
    detected: bool,
    universal: bool,
    global_dir: String,
    skills: String,
}

#[derive(Debug, Clone)]
pub struct AuditTarget {
    pub name: String,
    /// `None` for skills not installed from skills.sh.
    pub source: Option<String>,
    pub slug: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Bar {
    label: String,
    value: String,
    fraction: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentRow {
    name: String,
    source: String,
    when: String,
}

/// A line of the Overview's "Needs attention" list. `kind` is a status mark:
/// 0 fine, 1 warning, 2 failure, 3 neutral.
#[derive(Debug, Clone, Serialize)]
pub struct Attention {
    pub text: String,
    pub kind: u8,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    skills: String,
    skills_note: String,
    agents: String,
    agents_note: String,
    disk: String,
    disk_note: String,
    updated: String,
    updated_note: String,
    per_agent: Vec<Bar>,
    per_agent_note: String,
    per_source: Vec<Bar>,
    per_source_note: String,
    recent: Vec<RecentRow>,
    pub attention: Vec<Attention>,
}

/// Everything the frontend shows about the local library.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    overview: Overview,
    skills: Vec<SkillRow>,
    status: String,
    agents: Vec<AgentRow>,
    detected_agents: usize,
    health: HealthView,
}

pub fn library_view(st: &State) -> LibraryView {
    let lib = &st.library;
    let skills = lib
        .rows
        .iter()
        .map(|row| SkillRow {
            update: st
                .updates
                .results
                .get(&row.key)
                .map(|u| u.badge())
                .unwrap_or_default()
                .to_owned(),
            ..row.clone()
        })
        .collect();
    LibraryView {
        overview: lib.overview.clone(),
        skills,
        status: lib.status.clone(),
        agents: lib.agents.clone(),
        detected_agents: lib.detected,
        health: health_view(st),
    }
}

/// Scans the global skills and returns the refreshed library.
#[tauri::command]
pub async fn refresh(state: tauri::State<'_, AppState>) -> Result<LibraryView, String> {
    let env = state.env()?;
    let library = blocking(move || scan(&env)).await?;
    let mut guard = state.lock();
    let st = &mut *guard;
    st.library = library;
    // Update results for skills that are gone no longer apply.
    let names: HashSet<&str> = st
        .library
        .skills
        .iter()
        .map(|s| s.meta.name.as_str())
        .collect();
    st.updates.results.retain(|k, _| names.contains(k.as_str()));
    Ok(library_view(st))
}

fn scan(env: &Env) -> LibraryState {
    let report = scan_scope(env, &Scope::Global);
    let detected = detected_agents(env, None);
    let (lock, lock_note) = match read_global(env) {
        Ok(LockState::Loaded(lock)) => (Some(lock), None),
        Ok(LockState::Missing) => (None, None),
        Ok(LockState::Outdated { version }) => (
            None,
            Some(Attention {
                text: format!(
                    "Your skills lockfile is from an older skills CLI (v{version}), so update history isn't available. SkillsMate leaves it untouched."
                ),
                kind: 1,
            }),
        ),
        Ok(LockState::Newer { version }) => (
            None,
            Some(Attention {
                text: format!(
                    "Your skills lockfile is from a newer skills CLI (v{version}). Update SkillsMate to read its history."
                ),
                kind: 1,
            }),
        ),
        Err(e) => (
            None,
            Some(Attention {
                text: format!("Couldn't read the skills lockfile: {e}"),
                kind: 2,
            }),
        ),
    };

    let facts: Vec<SkillFacts> = report
        .skills
        .iter()
        .map(|s| skill_facts(s, lock.as_ref()))
        .collect();
    let summary = summarize(
        &report.skills,
        &facts,
        report.problems.len(),
        detected.len(),
        display_name,
    );
    let today = today();

    let rows = report
        .skills
        .iter()
        .zip(&facts)
        .map(|(skill, f)| {
            let mut agents: Vec<&str> = skill
                .installs
                .iter()
                .filter(|i| i.agent != "universal")
                .map(|i| display_name(i.agent))
                .collect();
            agents.sort_unstable_by_key(|a| a.to_lowercase());
            agents.dedup();
            let visible = !agents.is_empty();
            let agents = match agents.len() {
                0 => "Not visible to any detected agent".to_owned(),
                n if n <= 4 => agents.join(" · "),
                n => format!("{} · +{} more", agents[..3].join(" · "), n - 3),
            };
            let updated = f
                .updated_at
                .as_deref()
                .and_then(|d| relative_date(d, today))
                .map(|r| format!("updated {r}"))
                .unwrap_or_else(|| "not tracked".into());
            SkillRow {
                key: skill.meta.name.clone(),
                name: skill.meta.display_name(),
                description: skill.meta.description.clone(),
                agents,
                source: f.source.clone().unwrap_or_else(|| "Local".into()),
                meta: format!("{} · {updated}", format_bytes(f.size_bytes)),
                update: String::new(),
                managed: f.source.is_some(),
                visible,
            }
        })
        .collect();

    let audit_targets = report
        .skills
        .iter()
        .zip(&facts)
        .map(|(skill, f)| AuditTarget {
            name: skill.meta.display_name(),
            source: f.source.clone(),
            slug: skill.dir_name.clone(),
        })
        .collect();

    let installed = report
        .skills
        .iter()
        .zip(&facts)
        .filter_map(|(skill, f)| Some((f.source.clone()?, skill)))
        .flat_map(|(src, skill)| {
            [
                (src.clone(), skill.meta.name.clone()),
                (src, skill.dir_name.clone()),
            ]
        })
        .collect();

    let detected_ids: Vec<&str> = detected.iter().map(|a| a.id()).collect();
    // (skills this agent sees, row), sorted: detected first, then most skills.
    let mut agents: Vec<(usize, AgentRow)> = Agent::all()
        .filter(|a| a.id() != "universal")
        .map(|a| {
            let global_dir = a
                .skills_dir(env, &Scope::Global)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "No global skills directory".into());
            let skills = summary
                .per_agent
                .iter()
                .find(|(id, _)| *id == a.id())
                .map_or(0, |(_, n)| *n);
            let row = AgentRow {
                name: a.display_name().to_owned(),
                id: a.id().to_owned(),
                detected: detected_ids.contains(&a.id()),
                universal: a.is_universal(),
                global_dir,
                skills: if skills == 0 {
                    String::new()
                } else {
                    plural(skills, "skill", "skills")
                },
            };
            (skills, row)
        })
        .collect();
    agents.sort_by(|(an, a), (bn, b)| {
        b.detected
            .cmp(&a.detected)
            .then_with(|| bn.cmp(an))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let mut status = match summary.total {
        0 => "No global skills found".to_owned(),
        n => plural(n, "global skill", "global skills"),
    };
    if summary.problems > 0 {
        status.push_str(&format!(" · {} could not be read", summary.problems));
    }

    let health = health::check(&report, env.home(), display_name);
    let mut overview = overview(&summary, today, lock_note);
    health_attention(&mut overview.attention, &health);

    LibraryState {
        rows,
        skills: report.skills,
        facts,
        health,
        overview,
        audit_targets,
        installed,
        agents: agents.into_iter().map(|(_, row)| row).collect(),
        detected: detected.len(),
        status,
    }
}

/// Points the Overview at the Health page when it found something wrong, so
/// "No problems found" never sits beside a Health badge.
fn health_attention(attention: &mut Vec<Attention>, health: &HealthReport) {
    let problems = health.count(Severity::Problem);
    let warnings = health.count(Severity::Warning);
    let mut parts = Vec::new();
    if problems > 0 {
        parts.push(plural(problems, "problem", "problems"));
    }
    if warnings > 0 {
        parts.push(plural(warnings, "warning", "warnings"));
    }
    if parts.is_empty() {
        return;
    }
    attention.retain(|a| a.text != "No problems found.");
    attention.insert(
        0,
        Attention {
            text: format!("Health found {}.", parts.join(" and ")),
            kind: if problems > 0 { 2 } else { 1 },
        },
    );
}

pub fn display_name(id: &str) -> &'static str {
    Agent::by_id(id).map_or("Unknown agent", Agent::display_name)
}

fn overview(s: &Summary, today: i64, lock_note: Option<Attention>) -> Overview {
    let skills_note = if s.total == 0 {
        "Nothing installed yet".to_owned()
    } else {
        format!("{} from skills.sh · {} local", s.managed, s.unmanaged)
    };
    let updated = s
        .recent
        .first()
        .and_then(|r| Some((relative_date(&r.updated_at, today)?, r.name.clone())));
    let (updated, updated_note) =
        updated.unwrap_or_else(|| ("—".into(), "No tracked updates".into()));

    let agent_max = s.per_agent.first().map_or(1, |(_, n)| *n).max(1);
    let per_agent = s
        .per_agent
        .iter()
        .take(MAX_AGENT_BARS)
        .map(|(id, n)| Bar {
            label: display_name(id).to_owned(),
            value: plural(*n, "skill", "skills"),
            fraction: *n as f32 / agent_max as f32,
        })
        .collect();
    let per_agent_note = match s.per_agent.len() {
        n if n > MAX_AGENT_BARS => format!("top {MAX_AGENT_BARS} of {n} agents"),
        n => plural(n, "agent", "agents"),
    };

    let source_max = s.per_source.first().map_or(1, |c| c.count).max(1);
    let per_source = s
        .per_source
        .iter()
        .take(MAX_SOURCE_BARS)
        .map(|c| Bar {
            label: c.label.clone(),
            value: plural(c.count, "skill", "skills"),
            fraction: c.count as f32 / source_max as f32,
        })
        .collect();
    let per_source_note = match s.per_source.len() {
        n if n > MAX_SOURCE_BARS => format!("top {MAX_SOURCE_BARS} of {n} sources"),
        n => plural(n, "source", "sources"),
    };

    let recent = s
        .recent
        .iter()
        .map(|r| RecentRow {
            name: r.name.clone(),
            source: r.source.clone(),
            when: relative_date(&r.updated_at, today).unwrap_or_default(),
        })
        .collect();

    Overview {
        skills: s.total.to_string(),
        skills_note,
        agents: s.agents_in_use.to_string(),
        agents_note: format!("of {} detected", s.detected_agents),
        disk: format_bytes(s.disk_bytes),
        disk_note: format!(
            "{} · symlinks not counted",
            plural(s.folders, "folder", "folders")
        ),
        updated,
        updated_note,
        per_agent,
        per_agent_note,
        per_source,
        per_source_note,
        recent,
        attention: attention(s, lock_note),
    }
}

fn attention(s: &Summary, lock_note: Option<Attention>) -> Vec<Attention> {
    let row = |text: String, kind: u8| Attention { text, kind };
    let mut rows: Vec<Attention> = Vec::new();
    if s.detected_agents == 0 {
        rows.push(row("No coding agents detected on this computer.".into(), 1));
    }
    if s.unlinked > 0 {
        rows.push(row(
            format!(
                "{} not visible to any detected agent.",
                plural(s.unlinked, "skill is", "skills are")
            ),
            1,
        ));
    }
    if s.problems > 0 {
        rows.push(row(
            format!(
                "{} an invalid SKILL.md and can't be loaded.",
                plural(s.problems, "skill folder has", "skill folders have")
            ),
            1,
        ));
    }
    if let Some(note) = lock_note {
        rows.push(note);
    }
    if !rows.iter().any(|r| r.kind == 1 || r.kind == 2) {
        rows.insert(0, row("No problems found.".into(), 0));
    }
    if s.unmanaged > 0 {
        rows.push(row(
            format!(
                "{} installed outside skills.sh, so updates and audits aren't available for {}.",
                plural(s.unmanaged, "skill was", "skills were"),
                if s.unmanaged == 1 { "it" } else { "them" }
            ),
            3,
        ));
    }
    if s.total == 0 {
        rows.push(row("No skills installed yet.".into(), 3));
    }
    rows
}
