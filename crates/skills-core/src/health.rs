//! Local health checks: problems and improvements found without the network.
//!
//! Rules follow the Agent Skills specification (agentskills.io/specification):
//! `name` is 1–64 chars of `a-z`, `0-9` and single hyphens and matches its
//! folder; `description` is 1–1024 chars and says what the skill does and when
//! to use it; the SKILL.md body should stay under ~5,000 tokens / 500 lines,
//! because agents load all of it when the skill activates. Every skill's name
//! and description (~100 tokens) are loaded at the start of every session.
//!
//! Blocking (reads skill folders); run it off the UI thread.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use crate::agents::AgentId;
use crate::files::{read_skill_files, script_files, FileMap};
use crate::scan::{InstalledSkill, LinkKind, ScanReport};

/// Approximate tokens for English text (~4 characters per token).
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

const BODY_TOKEN_LIMIT: usize = 5_000;
const BODY_LINE_LIMIT: usize = 500;
const DESCRIPTION_MAX: usize = 1_024;
const DESCRIPTION_SHORT: usize = 40;
const OVERLAP_THRESHOLD: f32 = 0.5;
/// Report always-loaded metadata above this many tokens for one agent.
const STARTUP_TOKENS_NOTE: usize = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Something is broken.
    Problem,
    /// Works, but likely not as intended.
    Warning,
    /// An improvement.
    Tip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fix {
    /// Remove a dead symlink.
    RemoveBrokenLink,
    /// Replace an out-of-date copy with a link to the main copy.
    RelinkCopy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    /// Skill the finding is about (frontmatter name), if any.
    pub skill: Option<String>,
    pub title: String,
    pub detail: String,
    /// A path the fix acts on, for findings with a one-click fix.
    pub path: Option<PathBuf>,
    pub fix: Option<Fix>,
}

impl Finding {
    fn new(severity: Severity, skill: Option<&str>, title: String, detail: String) -> Self {
        Self {
            severity,
            skill: skill.map(str::to_owned),
            title,
            detail,
            path: None,
            fix: None,
        }
    }
}

/// Per-skill numbers the detail view shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkillProfile {
    pub files: usize,
    pub body_tokens: usize,
    pub body_lines: usize,
    /// Tokens loaded at the start of every session (name + description).
    pub startup_tokens: usize,
    pub scripts: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HealthReport {
    pub findings: Vec<Finding>,
    /// Same order as the scanned skills.
    pub profiles: Vec<SkillProfile>,
    /// Startup tokens per agent, largest first.
    pub startup_by_agent: Vec<(AgentId, usize, usize)>,
}

impl HealthReport {
    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

/// Shows a path relative to the home folder (`~/.claude/skills/x`).
pub fn pretty_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !rest.as_os_str().is_empty() => {
            format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
        }
        _ => path.display().to_string(),
    }
}

pub fn check(
    report: &ScanReport,
    home: &Path,
    display_name: impl Fn(AgentId) -> &'static str,
) -> HealthReport {
    let mut findings = Vec::new();
    let mut profiles = Vec::new();

    for p in &report.problems {
        let folder = p.path.file_name().map_or_else(
            || p.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        findings.push(Finding::new(
            Severity::Problem,
            None,
            format!("{folder} can't be loaded"),
            format!(
                "Its SKILL.md is invalid ({}), so agents skip it. Location: {}",
                p.reason,
                pretty_path(&p.path, home)
            ),
        ));
    }

    for link in &report.broken_links {
        let agents = if link.agents.is_empty() {
            "the main skills folder".to_owned()
        } else {
            link.agents
                .iter()
                .map(|a| display_name(a))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut f = Finding::new(
            Severity::Warning,
            None,
            "Broken link".into(),
            format!(
                "{} points to a skill that no longer exists ({agents}).",
                pretty_path(&link.path, home)
            ),
        );
        f.path = Some(link.path.clone());
        f.fix = Some(Fix::RemoveBrokenLink);
        findings.push(f);
    }

    for skill in &report.skills {
        let files = read_skill_files(skill.primary_path()).unwrap_or_default();
        let profile = profile(skill, &files);
        check_skill(skill, &files, &profile, &display_name, &mut findings);
        profiles.push(profile);
    }

    check_overlaps(&report.skills, &mut findings);

    // Always-loaded metadata per agent.
    let mut by_agent: Vec<(AgentId, usize, usize)> = Vec::new();
    for (skill, profile) in report.skills.iter().zip(&profiles) {
        let agents: BTreeSet<AgentId> = skill
            .installs
            .iter()
            .map(|i| i.agent)
            .filter(|a| *a != "universal")
            .collect();
        for a in agents {
            match by_agent.iter_mut().find(|(id, _, _)| *id == a) {
                Some(entry) => {
                    entry.1 += profile.startup_tokens;
                    entry.2 += 1;
                }
                None => by_agent.push((a, profile.startup_tokens, 1)),
            }
        }
    }
    by_agent.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    if let Some((agent, tokens, n)) = by_agent.first() {
        if *tokens > STARTUP_TOKENS_NOTE {
            findings.push(Finding::new(
                Severity::Tip,
                None,
                format!("{} starts every session with ~{} tokens of skills", display_name(agent), round_k(*tokens)),
                format!(
                    "Agents read every skill's name and description up front. {n} skills compete for {}'s attention; removing ones you don't use keeps it focused and leaves more room for your work.",
                    display_name(agent)
                ),
            ));
        }
    }

    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.skill.cmp(&b.skill))
            .then_with(|| a.title.cmp(&b.title))
    });
    HealthReport {
        findings,
        profiles,
        startup_by_agent: by_agent,
    }
}

fn profile(skill: &InstalledSkill, files: &FileMap) -> SkillProfile {
    SkillProfile {
        files: files.len(),
        body_tokens: estimate_tokens(&skill.meta.body),
        body_lines: skill.meta.body.lines().count(),
        startup_tokens: estimate_tokens(&skill.meta.name)
            + estimate_tokens(&skill.meta.description)
            + 10,
        scripts: script_files(files),
    }
}

fn check_skill(
    skill: &InstalledSkill,
    files: &FileMap,
    profile: &SkillProfile,
    display_name: &impl Fn(AgentId) -> &'static str,
    out: &mut Vec<Finding>,
) {
    let name = skill.meta.display_name();
    let n = Some(skill.meta.name.as_str());

    let visible = skill.installs.iter().any(|i| i.agent != "universal");
    if !visible {
        out.push(Finding::new(
            Severity::Warning,
            n,
            "Not visible to any agent".into(),
            format!("No detected agent reads the folder {name} is in, so it's never used."),
        ));
    }

    // Copies that drifted from the main copy.
    if let Some(canonical) = &skill.canonical_path {
        for install in skill.installs.iter().filter(|i| i.kind == LinkKind::Copy) {
            if install.path == *canonical {
                continue;
            }
            let copy = read_skill_files(&install.path).unwrap_or_default();
            if copy != *files {
                let mut f = Finding::new(
                    Severity::Warning,
                    n,
                    format!("{}'s copy is out of sync", display_name(install.agent)),
                    format!(
                        "{} uses its own copy of {name} that differs from the main copy, so it behaves differently from your other agents.",
                        display_name(install.agent)
                    ),
                );
                f.path = Some(install.path.clone());
                f.fix = Some(Fix::RelinkCopy);
                out.push(f);
            }
        }
    }

    // Name rules.
    if let Some(problem) = name_problem(&skill.meta.name) {
        out.push(Finding::new(
            Severity::Tip,
            n,
            "Name doesn't follow the Agent Skills spec".into(),
            format!("\"{name}\" {problem}. Some agents may refuse or rename it."),
        ));
    } else if skill.meta.name != skill.dir_name {
        out.push(Finding::new(
            Severity::Tip,
            n,
            "Name doesn't match its folder".into(),
            format!(
                "The skill is called \"{name}\" but lives in \"{}\". The spec requires them to match.",
                skill.dir_name
            ),
        ));
    }

    // Description quality: it's how agents decide when to use the skill.
    let desc = &skill.meta.description;
    let desc_len = desc.chars().count();
    if desc_len > DESCRIPTION_MAX {
        out.push(Finding::new(
            Severity::Warning,
            n,
            "Description is too long".into(),
            format!("{desc_len} characters; the limit is {DESCRIPTION_MAX}. Some agents truncate or reject it."),
        ));
    } else if desc_len < DESCRIPTION_SHORT {
        out.push(Finding::new(
            Severity::Tip,
            n,
            "Description is very short".into(),
            format!("\"{desc}\" gives agents little to go on, so {name} may not be used when it should be."),
        ));
    } else if !says_when(desc) {
        out.push(Finding::new(
            Severity::Tip,
            n,
            "Description doesn't say when to use it".into(),
            format!("Agents pick skills from their description. Adding \"Use when…\" helps them choose {name} at the right moment."),
        ));
    }

    // Context cost when the skill activates.
    if profile.body_tokens > BODY_TOKEN_LIMIT || profile.body_lines > BODY_LINE_LIMIT {
        out.push(Finding::new(
            Severity::Tip,
            n,
            "Large SKILL.md".into(),
            format!(
                "~{} tokens, {} lines. Agents load all of it each time {name} activates; the spec suggests under ~5k tokens and 500 lines, with details moved to reference files.",
                round_k(profile.body_tokens),
                profile.body_lines
            ),
        ));
    }

    if !profile.scripts.is_empty() {
        let list = profile
            .scripts
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = profile.scripts.len().saturating_sub(3);
        let more = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        let tools = skill
            .meta
            .allowed_tools
            .as_deref()
            .map(|t| format!(" It pre-approves these tools: {t}."))
            .unwrap_or_default();
        out.push(Finding::new(
            Severity::Tip,
            n,
            "Includes code the agent can run".into(),
            format!("{list}{more}.{tools} Review scripts before trusting a skill from someone you don't know."),
        ));
    }
}

/// Why `name` breaks the spec's rules, if it does.
pub fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() || name.chars().count() > 64 {
        return Some("must be 1–64 characters");
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Some("should use only lowercase letters, numbers and hyphens");
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Some("can't start or end with a hyphen or contain \"--\"");
    }
    None
}

fn says_when(desc: &str) -> bool {
    let d = desc.to_lowercase();
    [
        "when",
        "use for",
        "use this",
        "use it",
        "trigger",
        "if the user",
        "whenever",
    ]
    .iter()
    .any(|cue| d.contains(cue))
}

const STOPWORDS: [&str; 40] = [
    "the", "and", "for", "with", "use", "when", "this", "that", "your", "you", "are", "from",
    "into", "using", "used", "user", "users", "skill", "skills", "about", "any", "all", "can",
    "will", "should", "their", "them", "they", "how", "what", "which", "who", "help", "helps",
    "work", "working", "based", "such", "also", "more",
];

fn keywords(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(w))
        .map(|w| w.trim_end_matches('s').to_owned())
        .collect()
}

/// Skills whose descriptions share most of their keywords: an agent may pick
/// the wrong one, or load both.
fn check_overlaps(skills: &[InstalledSkill], out: &mut Vec<Finding>) {
    let words: Vec<HashSet<String>> = skills
        .iter()
        .map(|s| keywords(&s.meta.description))
        .collect();
    for i in 0..skills.len() {
        for j in i + 1..skills.len() {
            let (a, b) = (&words[i], &words[j]);
            if a.len() < 5 || b.len() < 5 {
                continue;
            }
            let inter = a.intersection(b).count() as f32;
            let union = a.union(b).count() as f32;
            if union > 0.0 && inter / union >= OVERLAP_THRESHOLD {
                let (x, y) = (skills[i].meta.display_name(), skills[j].meta.display_name());
                out.push(Finding::new(
                    Severity::Tip,
                    Some(&skills[i].meta.name),
                    format!("{x} and {y} overlap"),
                    "Their descriptions are very similar, so agents may pick the wrong one. Keep the one you prefer, or make each description more specific.".to_owned(),
                ));
            }
        }
    }
}

/// `6234` → `6.2k`, `850` → `850`.
pub fn round_k(n: usize) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        let s = format!("{:.1}", n as f64 / 1000.0);
        format!("{}k", s.trim_end_matches(".0"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Scope;
    use crate::scan::{AgentInstall, BrokenLink};
    use crate::skill_md::SkillMeta;

    fn skill(name: &str, desc: &str, body: &str, agents: &[AgentId]) -> InstalledSkill {
        InstalledSkill {
            meta: SkillMeta {
                name: name.into(),
                description: desc.into(),
                body: body.into(),
                ..SkillMeta::default()
            },
            scope: Scope::Global,
            dir_name: name.into(),
            canonical_path: None,
            installs: agents
                .iter()
                .map(|a| AgentInstall {
                    agent: a,
                    path: PathBuf::from(format!("/nonexistent/{a}/{name}")),
                    kind: LinkKind::Symlink,
                })
                .collect(),
        }
    }

    fn titles(r: &HealthReport) -> Vec<String> {
        r.findings.iter().map(|f| f.title.clone()).collect()
    }

    const GOOD: &str = "Extracts text and tables from PDF files and fills forms. Use when working with PDF documents.";

    #[test]
    fn healthy_skill_has_no_findings() {
        let report = ScanReport {
            skills: vec![skill("pdf", GOOD, "# PDF\n", &["codex"])],
            ..ScanReport::default()
        };
        let r = check(&report, Path::new("/home/u"), |_| "Agent");
        assert!(r.findings.is_empty(), "{:?}", titles(&r));
        assert_eq!(r.profiles[0].body_lines, 1);
    }

    #[test]
    fn flags_spec_and_quality_issues() {
        let big = "word ".repeat(5000);
        let report = ScanReport {
            skills: vec![
                skill("Bad_Name", "Helps with PDFs.", "", &["codex"]),
                skill("orphan", GOOD, &big, &[]),
                skill("verbose", &"x".repeat(1100), "", &["codex"]),
                skill(
                    "nowhen",
                    "Formats markdown tables with aligned columns and consistent padding.",
                    "",
                    &["codex"],
                ),
            ],
            broken_links: vec![BrokenLink {
                path: PathBuf::from("/a/.claude/skills/gone"),
                agents: vec!["claude-code"],
            }],
            ..ScanReport::default()
        };
        let r = check(&report, Path::new("/home/u"), |_| "Agent");
        let t = titles(&r);
        for expected in [
            "Name doesn't follow the Agent Skills spec",
            "Description is very short",
            "Not visible to any agent",
            "Large SKILL.md",
            "Description is too long",
            "Description doesn't say when to use it",
            "Broken link",
        ] {
            assert!(t.iter().any(|x| x == expected), "missing {expected}: {t:?}");
        }
        // Warnings sort before tips.
        assert_eq!(r.findings[0].severity, Severity::Warning);
        let broken = r
            .findings
            .iter()
            .find(|f| f.title == "Broken link")
            .expect("broken");
        assert_eq!(broken.fix, Some(Fix::RemoveBrokenLink));
    }

    #[test]
    fn detects_overlapping_descriptions() {
        let report = ScanReport {
            skills: vec![
                skill("react-perf", "React performance rules for components, hooks, rendering and memoization. Use when optimizing React apps.", "", &["codex"]),
                skill("react-speed", "Performance rules for React components, hooks, memoization and rendering. Use when making React apps faster.", "", &["codex"]),
                skill("pdf", GOOD, "", &["codex"]),
            ],
            ..ScanReport::default()
        };
        let r = check(&report, Path::new("/home/u"), |_| "Agent");
        assert!(
            titles(&r).contains(&"react-perf and react-speed overlap".to_owned()),
            "{:?}",
            titles(&r)
        );
        assert_eq!(
            titles(&r).iter().filter(|t| t.contains("overlap")).count(),
            1
        );
    }

    #[test]
    fn drifted_copy_is_flagged() {
        let home = tempfile::tempdir().expect("tempdir");
        let main = home.path().join("main");
        let copy = home.path().join("copy");
        for (dir, body) in [(&main, "v2"), (&copy, "v1")] {
            std::fs::create_dir_all(dir).expect("mkdir");
            std::fs::write(dir.join("SKILL.md"), body).expect("write");
        }
        let mut s = skill("pdf", GOOD, "", &[]);
        s.canonical_path = Some(main);
        s.installs = vec![AgentInstall {
            agent: "windsurf",
            path: copy.clone(),
            kind: LinkKind::Copy,
        }];
        let r = check(
            &ScanReport {
                skills: vec![s],
                ..ScanReport::default()
            },
            home.path(),
            |_| "Windsurf",
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.title == "Windsurf's copy is out of sync")
            .expect("drift finding");
        assert_eq!(f.fix, Some(Fix::RelinkCopy));
        assert_eq!(f.path.as_deref(), Some(copy.as_path()));
    }

    #[test]
    fn spec_name_rules() {
        assert_eq!(name_problem("pdf-processing"), None);
        assert!(name_problem("PDF").is_some());
        assert!(name_problem("-pdf").is_some());
        assert!(name_problem("pdf--x").is_some());
        assert!(name_problem(&"a".repeat(65)).is_some());
        assert_eq!(round_k(6234), "6.2k");
        assert_eq!(round_k(2000), "2k");
        #[cfg(unix)]
        assert_eq!(
            pretty_path(Path::new("/home/u/.claude/skills/x"), Path::new("/home/u")),
            "~/.claude/skills/x"
        );
    }
}
