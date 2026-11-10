//! Local library: scanning installed skills and building the Overview,
//! Installed and Agents pages.

use std::collections::HashSet;

use skills_core::agents::{detected_agents, Agent};
use skills_core::health::{self, HealthReport};
use skills_core::lock::{read_global, LockState};
use skills_core::scan::{scan_scope, InstalledSkill};
use skills_core::stats::{
    format_bytes, relative_date, skill_facts, summarize, today, SkillFacts, Summary,
};
use skills_core::{Env, Scope};

use super::{model, plural, to_i32, Controller};
use crate::{AgentRow, AttentionRow, BarRow, Overview, RecentRow, SkillRow};

const MAX_AGENT_BARS: usize = 8;
const MAX_SOURCE_BARS: usize = 6;

/// What the UI thread keeps from the last scan.
#[derive(Default)]
pub(super) struct LibraryState {
    pub items: Vec<SkillItem>,
    /// The scanned skills, their facts and health, in the same order.
    pub skills: Vec<InstalledSkill>,
    pub facts: Vec<SkillFacts>,
    pub health: HealthReport,
    pub overview: OverviewData,
    /// Installed skills to check for audits.
    pub audit_targets: Vec<AuditTarget>,
    /// `(source, name)` and `(source, dir name)` of managed skills, for
    /// "Installed" markers on directory pages.
    pub installed: HashSet<(String, String)>,
}

#[derive(Debug, Clone)]
pub(super) struct SkillItem {
    pub key: String,
    name: String,
    description: String,
    agents: String,
    source: String,
    meta: String,
}

impl SkillItem {
    fn matches(&self, needle: &str) -> bool {
        needle.is_empty()
            || [&self.name, &self.description, &self.source, &self.agents]
                .iter()
                .any(|f| f.to_lowercase().contains(needle))
    }

    fn to_row(&self, update: &str) -> SkillRow {
        SkillRow {
            key: self.key.as_str().into(),
            name: self.name.as_str().into(),
            description: self.description.as_str().into(),
            agents: self.agents.as_str().into(),
            source: self.source.as_str().into(),
            meta: self.meta.as_str().into(),
            update: update.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct AuditTarget {
    pub name: String,
    /// `None` for skills not installed from skills.sh.
    pub source: Option<String>,
    pub slug: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Bar {
    label: String,
    value: String,
    fraction: f32,
}

#[derive(Debug, Clone, Default)]
pub(super) struct OverviewData {
    tiles: [(String, String); 4],
    per_agent: Vec<Bar>,
    per_agent_note: String,
    per_source: Vec<Bar>,
    per_source_note: String,
    recent: Vec<(String, String, String)>,
    pub attention: Vec<(String, i32)>,
}

impl OverviewData {
    pub fn to_ui(&self) -> Overview {
        let bars = |bars: &[Bar]| {
            model(
                bars.iter()
                    .map(|b| BarRow {
                        label: b.label.as_str().into(),
                        value: b.value.as_str().into(),
                        fraction: b.fraction,
                    })
                    .collect(),
            )
        };
        let t = &self.tiles;
        Overview {
            skills: t[0].0.as_str().into(),
            skills_note: t[0].1.as_str().into(),
            agents: t[1].0.as_str().into(),
            agents_note: t[1].1.as_str().into(),
            disk: t[2].0.as_str().into(),
            disk_note: t[2].1.as_str().into(),
            updated: t[3].0.as_str().into(),
            updated_note: t[3].1.as_str().into(),
            per_agent: bars(&self.per_agent),
            per_agent_note: self.per_agent_note.as_str().into(),
            per_source: bars(&self.per_source),
            per_source_note: self.per_source_note.as_str().into(),
            recent: model(
                self.recent
                    .iter()
                    .map(|(name, source, when)| RecentRow {
                        name: name.as_str().into(),
                        source: source.as_str().into(),
                        when: when.as_str().into(),
                    })
                    .collect(),
            ),
            attention: model(
                self.attention
                    .iter()
                    .map(|(text, kind)| AttentionRow {
                        text: text.as_str().into(),
                        kind: *kind,
                    })
                    .collect(),
            ),
        }
    }
}

/// Everything one scan produces, built off the UI thread.
struct Scan {
    items: Vec<SkillItem>,
    skills: Vec<InstalledSkill>,
    facts: Vec<SkillFacts>,
    health: HealthReport,
    overview: OverviewData,
    agents: Vec<(bool, String, String, bool, String, usize)>,
    detected: usize,
    audit_targets: Vec<AuditTarget>,
    installed: HashSet<(String, String)>,
    status: String,
}

impl Controller {
    pub(super) fn refresh_library(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_loading(true);
        }
        let env = std::sync::Arc::clone(&self.env);
        let state = std::rc::Rc::clone(&self.state);
        let this = self.clone();
        self.run_blocking(
            move || scan(&env),
            move |ui, scan| {
                {
                    let mut st = state.borrow_mut();
                    st.library = LibraryState {
                        items: scan.items,
                        skills: scan.skills,
                        facts: scan.facts,
                        health: scan.health,
                        overview: scan.overview,
                        audit_targets: scan.audit_targets,
                        installed: scan.installed,
                    };
                    // Update results for skills that are gone no longer apply.
                    let names: HashSet<String> = st
                        .library
                        .skills
                        .iter()
                        .map(|s| s.meta.name.clone())
                        .collect();
                    st.updates.results.retain(|k, _| names.contains(k));
                    ui.set_overview(st.library.overview.to_ui());
                    ui.set_installed_total(to_i32(st.library.items.len()));
                }
                ui.set_installed_status(scan.status.into());
                let agents: Vec<AgentRow> = scan
                    .agents
                    .into_iter()
                    .map(|(detected, name, id, universal, dir, skills)| AgentRow {
                        name: name.into(),
                        id: id.into(),
                        detected,
                        universal,
                        global_dir: dir.into(),
                        skills: if skills == 0 {
                            "".into()
                        } else {
                            plural(skills, "skill", "skills").into()
                        },
                    })
                    .collect();
                ui.set_agents(model(agents));
                ui.set_detected_agents(to_i32(scan.detected));
                ui.set_loading(false);
                this.filter_installed(&ui.get_installed_filter());
                this.show_health();
                this.refresh_detail();
                if this.state.borrow().explore.has_recommendation_data() {
                    this.show_recommendations();
                }
                // Audit results describe the previous scan; drop them.
                ui.set_audits(model(Vec::new()));
                ui.set_audits_flagged(0);
                ui.set_audits_status(
                    "Checks your installed skills against skills.sh security audits".into(),
                );
            },
        );
    }

    pub(super) fn filter_installed(&self, filter: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let needle = filter.trim().to_lowercase();
        let st = self.state.borrow();
        let rows: Vec<SkillRow> = st
            .library
            .items
            .iter()
            .filter(|s| s.matches(&needle))
            .map(|s| {
                let update = st
                    .updates
                    .results
                    .get(&s.key)
                    .map(|u| u.badge())
                    .unwrap_or_default();
                s.to_row(update)
            })
            .collect();
        ui.set_installed(model(rows));
    }
}

fn scan(env: &Env) -> Scan {
    let report = scan_scope(env, &Scope::Global);
    let detected = detected_agents(env, None);
    let (lock, lock_note) = match read_global(env) {
        Ok(LockState::Loaded(lock)) => (Some(lock), None),
        Ok(LockState::Missing) => (None, None),
        Ok(LockState::Outdated { version }) => (
            None,
            Some((
                format!(
                    "Your skills lockfile is from an older skills CLI (v{version}), so update history isn't available. SkillsMate leaves it untouched."
                ),
                1,
            )),
        ),
        Ok(LockState::Newer { version }) => (
            None,
            Some((
                format!(
                    "Your skills lockfile is from a newer skills CLI (v{version}). Update SkillsMate to read its history."
                ),
                1,
            )),
        ),
        Err(e) => (None, Some((format!("Couldn't read the skills lockfile: {e}"), 2))),
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

    let items = report
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
            SkillItem {
                key: skill.meta.name.clone(),
                name: skill.meta.display_name(),
                description: skill.meta.description.clone(),
                agents,
                source: f.source.clone().unwrap_or_else(|| "Local".into()),
                meta: format!("{} · {updated}", format_bytes(f.size_bytes)),
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
    let mut agents: Vec<(bool, String, String, bool, String, usize)> = Agent::all()
        .filter(|a| a.id() != "universal")
        .map(|a| {
            let dir = a
                .skills_dir(env, &Scope::Global)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "No global skills directory".into());
            let skills = summary
                .per_agent
                .iter()
                .find(|(id, _)| *id == a.id())
                .map_or(0, |(_, n)| *n);
            (
                detected_ids.contains(&a.id()),
                a.display_name().to_owned(),
                a.id().to_owned(),
                a.is_universal(),
                dir,
                skills,
            )
        })
        .collect();
    agents.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.5.cmp(&a.5))
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });

    let mut status = match summary.total {
        0 => "No global skills found".to_owned(),
        n => plural(n, "global skill", "global skills"),
    };
    if summary.problems > 0 {
        status.push_str(&format!(" · {} could not be read", summary.problems));
    }

    let health = health::check(&report, env.home(), display_name);

    Scan {
        items,
        skills: report.skills,
        facts,
        health,
        overview: overview(&summary, today, lock_note),
        agents,
        detected: detected.len(),
        audit_targets,
        installed,
        status,
    }
}

pub(super) fn display_name(id: &str) -> &'static str {
    Agent::by_id(id).map_or("Unknown agent", Agent::display_name)
}

fn overview(s: &Summary, today: i64, lock_note: Option<(String, i32)>) -> OverviewData {
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
        .map(|r| {
            (
                r.name.clone(),
                r.source.clone(),
                relative_date(&r.updated_at, today).unwrap_or_default(),
            )
        })
        .collect();

    OverviewData {
        tiles: [
            (s.total.to_string(), skills_note),
            (
                s.agents_in_use.to_string(),
                format!("of {} detected", s.detected_agents),
            ),
            (
                format_bytes(s.disk_bytes),
                format!(
                    "{} · symlinks not counted",
                    plural(s.folders, "folder", "folders")
                ),
            ),
            (updated, updated_note),
        ],
        per_agent,
        per_agent_note,
        per_source,
        per_source_note,
        recent,
        attention: attention(s, lock_note),
    }
}

fn attention(s: &Summary, lock_note: Option<(String, i32)>) -> Vec<(String, i32)> {
    let mut rows: Vec<(String, i32)> = Vec::new();
    if s.detected_agents == 0 {
        rows.push(("No coding agents detected on this computer.".into(), 1));
    }
    if s.unlinked > 0 {
        rows.push((
            format!(
                "{} not visible to any detected agent.",
                plural(s.unlinked, "skill is", "skills are")
            ),
            1,
        ));
    }
    if s.problems > 0 {
        rows.push((
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
    if !rows.iter().any(|(_, k)| *k == 1 || *k == 2) {
        rows.insert(0, ("No problems found.".into(), 0));
    }
    if s.unmanaged > 0 {
        rows.push((
            format!(
                "{} installed outside skills.sh, so updates and audits aren't available for {}.",
                plural(s.unmanaged, "skill was", "skills were"),
                if s.unmanaged == 1 { "it" } else { "them" }
            ),
            3,
        ));
    }
    if s.total == 0 {
        rows.push(("No skills installed yet.".into(), 3));
    }
    rows
}
