//! The skill detail view.

use skills_core::directory::{to_skill_slug, SITE_BASE};
use skills_core::files::{read_skill_files, DiffLineKind};
use skills_core::health::{pretty_path, round_k, Finding, SkillProfile};
use skills_core::markdown::{blocks, BlockKind};
use skills_core::scan::{InstalledSkill, LinkKind};
use skills_core::stats::{format_bytes, relative_date, today, SkillFacts};

use super::health::{finding_kind, fix_label};
use super::library::display_name;
use super::{model, plural, to_i32, Controller};
use crate::{AgentUse, Detail, DiffRow, FileRow, FindingRow, MdBlock, Page};

/// Everything the detail view needs, gathered on the UI thread and turned
/// into a `Detail` after reading files off-thread.
struct Input {
    home: std::path::PathBuf,
    skill: InstalledSkill,
    facts: SkillFacts,
    profile: SkillProfile,
    findings: Vec<(usize, Finding)>,
    update: Option<UpdateView>,
}

/// The update check result as the detail view shows it: label, status kind,
/// summary, and diff lines as `(kind, text)`.
type UpdateView = (String, i32, String, Vec<(i32, String)>);

/// Plain, `Send` version of the view, built off the UI thread.
struct Built {
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
    update_kind: i32,
    update_summary: String,
    can_update: bool,
    url: String,
    license: String,
    compatibility: String,
    allowed_tools: String,
    agents: Vec<(String, String)>,
    findings: Vec<(usize, Finding)>,
    diff: Vec<(i32, String)>,
    body: Vec<(i32, String, i32)>,
    files: Vec<(String, String)>,
}

impl Controller {
    pub(super) fn open_skill(&self, key: String) {
        self.state.borrow_mut().detail_key = Some(key);
        self.refresh_detail();
    }

    /// Rebuilds the detail view for the open skill (after a scan or update
    /// check). Goes back if the skill no longer exists.
    pub(super) fn refresh_detail(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let input = {
            let st = self.state.borrow();
            let Some(key) = st.detail_key.clone() else {
                return;
            };
            let lib = &st.library;
            let Some(i) = lib.skills.iter().position(|s| s.meta.name == key) else {
                drop(st);
                self.state.borrow_mut().detail_key = None;
                if ui.get_page() == Page::Detail {
                    ui.set_page(ui.get_detail_back());
                }
                return;
            };
            let findings = lib
                .health
                .findings
                .iter()
                .enumerate()
                .filter(|(_, f)| f.skill.as_deref() == Some(key.as_str()))
                .map(|(i, f)| (i, f.clone()))
                .collect();
            let update = st.updates.results.get(&key).map(|u| {
                (
                    u.state.label().to_owned(),
                    u.kind(),
                    if u.diff.is_empty() {
                        String::new()
                    } else {
                        u.diff.summary()
                    },
                    u.diff
                        .lines
                        .iter()
                        .map(|l| {
                            let k = match l.kind {
                                DiffLineKind::File => 0,
                                DiffLineKind::Added => 1,
                                DiffLineKind::Removed => 2,
                                DiffLineKind::Context => 3,
                                DiffLineKind::Note => 4,
                            };
                            (k, l.text.clone())
                        })
                        .collect(),
                )
            });
            Input {
                home: self.env.home().to_path_buf(),
                skill: lib.skills[i].clone(),
                facts: lib.facts.get(i).cloned().unwrap_or_default(),
                profile: lib.health.profiles.get(i).cloned().unwrap_or_default(),
                findings,
                update,
            }
        };
        self.run_blocking(
            move || build(input),
            move |ui, b| {
                let findings: Vec<FindingRow> = b
                    .findings
                    .iter()
                    .map(|(i, f)| FindingRow {
                        title: f.title.as_str().into(),
                        detail: f.detail.as_str().into(),
                        skill: "".into(),
                        key: b.key.as_str().into(),
                        kind: finding_kind(f.severity),
                        fix_label: fix_label(f.fix).into(),
                        index: to_i32(*i),
                    })
                    .collect();
                ui.set_detail(Detail {
                    key: b.key.into(),
                    name: b.name.into(),
                    description: b.description.into(),
                    version: b.version.into(),
                    source: b.source.into(),
                    managed: b.managed,
                    location: b.location.into(),
                    size: b.size.into(),
                    files_count: b.files_count.into(),
                    active_tokens: b.active_tokens.into(),
                    startup_tokens: b.startup_tokens.into(),
                    update_label: b.update_label.into(),
                    update_kind: b.update_kind,
                    update_summary: b.update_summary.into(),
                    can_update: b.can_update,
                    url: b.url.into(),
                    license: b.license.into(),
                    compatibility: b.compatibility.into(),
                    allowed_tools: b.allowed_tools.into(),
                    agents: model(
                        b.agents
                            .into_iter()
                            .map(|(name, how)| AgentUse {
                                name: name.into(),
                                how: how.into(),
                            })
                            .collect(),
                    ),
                    findings: model(findings),
                    diff: model(
                        b.diff
                            .into_iter()
                            .map(|(kind, text)| DiffRow {
                                kind,
                                text: text.into(),
                            })
                            .collect(),
                    ),
                    body: model(
                        b.body
                            .into_iter()
                            .map(|(kind, text, depth)| MdBlock {
                                kind,
                                text: text.into(),
                                depth,
                            })
                            .collect(),
                    ),
                    files: model(
                        b.files
                            .into_iter()
                            .map(|(path, size)| FileRow {
                                path: path.into(),
                                size: size.into(),
                            })
                            .collect(),
                    ),
                });
            },
        );
    }
}

fn build(input: Input) -> Built {
    let Input {
        home,
        skill,
        facts,
        profile,
        findings,
        update,
    } = input;
    let files = read_skill_files(skill.primary_path()).unwrap_or_default();

    let mut agents: Vec<(String, String)> = skill
        .installs
        .iter()
        .filter(|i| i.agent != "universal")
        .map(|i| {
            let how = match i.kind {
                LinkKind::Canonical => "reads the main copy",
                LinkKind::Symlink => "linked to the main copy",
                LinkKind::Copy => "has its own copy",
            };
            (display_name(i.agent).to_owned(), how.to_owned())
        })
        .collect();
    agents.sort();
    agents.dedup();

    let today = today();
    let (update_label, update_kind, update_summary, diff, can_update) = match update {
        Some((label, kind, summary, diff)) => {
            let can = label != "Up to date";
            (label, kind, summary, diff, can)
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
        .map(|b| {
            let kind = match b.kind {
                BlockKind::Heading1 => 0,
                BlockKind::Heading2 => 1,
                BlockKind::Heading3 => 2,
                BlockKind::Paragraph => 3,
                BlockKind::Bullet => 4,
                BlockKind::Code => 5,
                BlockKind::Quote => 6,
            };
            (kind, b.text, to_i32(b.depth))
        })
        .collect();

    let file_rows = files
        .iter()
        .map(|(p, bytes)| (p.clone(), format_bytes(bytes.len() as u64)))
        .collect();

    Built {
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
        agents,
        findings,
        diff,
        body,
        files: file_rows,
    }
}
