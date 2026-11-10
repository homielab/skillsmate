//! Discovering installed skills on disk.
//!
//! Mirrors the CLI's `listInstalledSkills`: scan the canonical directory plus
//! every agent's skills directory (detected agents always, other agents when
//! their directory exists), then merge results by skill name within a scope.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::agents::{canonical_skills_dir, Agent, AgentId};
use crate::env::{Env, Scope};
use crate::sanitize::is_within;
use crate::skill_md::{read_skill_dir, SkillMeta, SKILL_FILE};

/// How a skill directory is materialised in an agent's skills folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// The agent reads the canonical directory itself.
    Canonical,
    /// A symlink (or Windows junction) pointing elsewhere.
    Symlink,
    /// An independent copy.
    Copy,
}

#[derive(Debug, Clone)]
pub struct AgentInstall {
    pub agent: AgentId,
    pub path: PathBuf,
    pub kind: LinkKind,
}

#[derive(Debug, Clone)]
pub struct InstalledSkill {
    pub meta: SkillMeta,
    pub scope: Scope,
    /// Directory name on disk (usually `sanitize_name(meta.name)`).
    pub dir_name: String,
    /// The canonical copy, if the skill is in `.agents/skills`.
    pub canonical_path: Option<PathBuf>,
    pub installs: Vec<AgentInstall>,
}

impl InstalledSkill {
    /// The directory to read the skill from: the canonical one if present.
    pub fn primary_path(&self) -> &Path {
        self.canonical_path
            .as_deref()
            .or_else(|| self.installs.first().map(|i| i.path.as_path()))
            .unwrap_or_else(|| Path::new(""))
    }
}

/// A skill directory that exists but could not be loaded.
#[derive(Debug, Clone)]
pub struct ScanProblem {
    pub path: PathBuf,
    pub reason: String,
}

/// A symlink in a skills folder whose target no longer exists.
#[derive(Debug, Clone)]
pub struct BrokenLink {
    pub path: PathBuf,
    /// Agents that read the folder the link is in.
    pub agents: Vec<AgentId>,
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub skills: Vec<InstalledSkill>,
    pub problems: Vec<ScanProblem>,
    pub broken_links: Vec<BrokenLink>,
}

/// Scans one scope for installed skills. Blocking; run it off the UI thread.
pub fn scan_scope(env: &Env, scope: &Scope) -> ScanReport {
    let project = match scope {
        Scope::Project(dir) => Some(dir.as_path()),
        Scope::Global => None,
    };
    let canonical = canonical_skills_dir(env, scope);
    let mut report = ScanReport::default();
    // Keyed by frontmatter name, sorted for stable output.
    let mut by_name: BTreeMap<String, InstalledSkill> = BTreeMap::new();
    // Cache directory listings: many agents share a directory.
    let mut listings: HashMap<PathBuf, Listing> = HashMap::new();

    let canonical_listing = list_dir(&canonical, &mut report.problems);
    for path in &canonical_listing.broken {
        record_broken(&mut report.broken_links, path, None);
    }
    for found in canonical_listing.found {
        let entry = by_name
            .entry(found.meta.name.clone())
            .or_insert_with(|| InstalledSkill {
                meta: found.meta.clone(),
                scope: scope.clone(),
                dir_name: found.dir_name.clone(),
                canonical_path: Some(found.path.clone()),
                installs: Vec::new(),
            });
        entry.canonical_path = Some(found.path.clone());
    }

    for agent in Agent::all() {
        let detected = agent.is_detected(env, project);
        let native = match scope {
            Scope::Global => agent.native_global_dir(env),
            Scope::Project(_) => None,
        };
        let dirs = agent.skills_dir(env, scope).into_iter().chain(native);

        for dir in dirs {
            if !detected && !dir.is_dir() {
                continue;
            }
            if dir == canonical {
                // Universal agents read the canonical dir; like the CLI, only
                // attribute skills to them when the agent is detected.
                if detected {
                    attribute_canonical(&mut by_name, agent.id());
                }
                continue;
            }
            let listing = listings
                .entry(dir.clone())
                .or_insert_with(|| list_dir(&dir, &mut report.problems));
            attribute_found(&mut by_name, &listing.found, scope, agent.id());
            for path in &listing.broken {
                record_broken(&mut report.broken_links, path, Some(agent.id()));
            }
        }
    }

    report.skills = by_name.into_values().collect();
    report
}

fn record_broken(out: &mut Vec<BrokenLink>, path: &Path, agent: Option<AgentId>) {
    match out.iter_mut().find(|b| b.path == path) {
        Some(b) => {
            if let Some(a) = agent {
                if !b.agents.contains(&a) {
                    b.agents.push(a);
                }
            }
        }
        None => out.push(BrokenLink {
            path: path.to_path_buf(),
            agents: agent.into_iter().collect(),
        }),
    }
}

fn attribute_canonical(by_name: &mut BTreeMap<String, InstalledSkill>, agent: AgentId) {
    for skill in by_name.values_mut() {
        let Some(path) = skill.canonical_path.clone() else {
            continue;
        };
        if !skill.installs.iter().any(|i| i.agent == agent) {
            skill.installs.push(AgentInstall {
                agent,
                path,
                kind: LinkKind::Canonical,
            });
        }
    }
}

fn attribute_found(
    by_name: &mut BTreeMap<String, InstalledSkill>,
    found: &[Found],
    scope: &Scope,
    agent: AgentId,
) {
    for f in found {
        let skill = by_name
            .entry(f.meta.name.clone())
            .or_insert_with(|| InstalledSkill {
                meta: f.meta.clone(),
                scope: scope.clone(),
                dir_name: f.dir_name.clone(),
                canonical_path: None,
                installs: Vec::new(),
            });
        if !skill.installs.iter().any(|i| i.agent == agent) {
            skill.installs.push(AgentInstall {
                agent,
                path: f.path.clone(),
                kind: if f.is_symlink {
                    LinkKind::Symlink
                } else {
                    LinkKind::Copy
                },
            });
        }
    }
}

struct Found {
    meta: SkillMeta,
    dir_name: String,
    path: PathBuf,
    is_symlink: bool,
}

#[derive(Default)]
struct Listing {
    found: Vec<Found>,
    broken: Vec<PathBuf>,
}

fn list_dir(dir: &Path, problems: &mut Vec<ScanProblem>) -> Listing {
    let mut out = Listing::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_within(dir, &path) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let is_symlink = file_type.is_symlink();
        // `metadata` follows symlinks, so links to directories count too.
        let target = path.metadata();
        if is_symlink && target.is_err() {
            out.broken.push(path);
            continue;
        }
        if !target.is_ok_and(|m| m.is_dir()) {
            continue;
        }
        if !path.join(SKILL_FILE).is_file() {
            continue;
        }
        match read_skill_dir(&path) {
            Ok(meta) => out.found.push(Found {
                meta,
                dir_name: entry.file_name().to_string_lossy().into_owned(),
                path,
                is_symlink,
            }),
            Err(e) => problems.push(ScanProblem {
                path,
                reason: match e {
                    crate::error::Error::InvalidSkill { reason, .. } => reason,
                    other => other.to_string(),
                },
            }),
        }
    }
    out
}
