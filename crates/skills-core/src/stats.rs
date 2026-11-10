//! Facts and summary statistics about installed skills, for the Overview page.
//!
//! [`skill_facts`] does filesystem I/O (folder sizes), so call it off the UI
//! thread. [`summarize`] is pure.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::agents::AgentId;
use crate::lock::GlobalLock;
use crate::scan::{InstalledSkill, LinkKind};

/// Stop walking a skill folder after this many entries. Real skills have a
/// handful of files; the cap keeps a pathological folder from stalling a scan.
const MAX_WALK_ENTRIES: usize = 10_000;
const MAX_WALK_DEPTH: usize = 16;

/// Per-skill facts that need I/O or the lockfile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkillFacts {
    /// Size of the skill's primary folder.
    pub size_bytes: u64,
    /// Space used by all real (non-symlink) copies of the skill.
    pub disk_bytes: u64,
    /// Number of real copies on disk.
    pub copies: usize,
    /// `owner/repo` (or similar) when the skill is tracked in the global lock.
    pub source: Option<String>,
    pub installed_at: Option<String>,
    pub updated_at: Option<String>,
}

pub fn skill_facts(skill: &InstalledSkill, lock: Option<&GlobalLock>) -> SkillFacts {
    // Real copies: the canonical folder plus any agent folder that is a copy.
    // Symlinks point at the canonical folder and take no extra space.
    let mut real: Vec<&Path> = Vec::new();
    if let Some(c) = &skill.canonical_path {
        real.push(c);
    }
    for i in &skill.installs {
        if i.kind == LinkKind::Copy && !real.contains(&i.path.as_path()) {
            real.push(&i.path);
        }
    }
    let sizes: Vec<u64> = real.iter().map(|p| dir_size(p)).collect();
    let entry = lock.and_then(|l| l.skills.get(&skill.meta.name));

    SkillFacts {
        size_bytes: sizes
            .first()
            .copied()
            .unwrap_or_else(|| dir_size(skill.primary_path())),
        disk_bytes: sizes.iter().sum(),
        copies: real.len(),
        source: entry.map(|e| e.source.clone()),
        installed_at: entry.map(|e| e.installed_at.clone()),
        updated_at: entry.map(|e| e.updated_at.clone()),
    }
}

/// Total size of regular files under `dir`, without following symlinks.
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((path, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_WALK_ENTRIES {
                return total;
            }
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if meta.is_file() {
                total = total.saturating_add(meta.len());
            } else if meta.is_dir() && depth < MAX_WALK_DEPTH {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    total
}

/// A labelled count, for ranked lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Count {
    pub label: String,
    pub count: usize,
}

/// A recently updated skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    pub name: String,
    pub source: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub total: usize,
    /// Tracked in the global lock, so `skills update` can update them.
    pub managed: usize,
    /// On disk but not in the lock (installed by hand or by an older CLI).
    pub unmanaged: usize,
    /// Not visible to any detected agent.
    pub unlinked: usize,
    /// Skill folders that exist but could not be read.
    pub problems: usize,
    pub detected_agents: usize,
    /// Agents that can see at least one skill.
    pub agents_in_use: usize,
    pub disk_bytes: u64,
    /// Real folders on disk, across all skills.
    pub folders: usize,
    /// Skills per agent, most first; ties by name.
    pub per_agent: Vec<(AgentId, usize)>,
    /// Managed skills per source, most first; ties by name.
    pub per_source: Vec<Count>,
    /// Up to five most recently updated managed skills, newest first.
    pub recent: Vec<Recent>,
}

/// Aggregates facts. `facts` must be in the same order as `skills`.
/// `display_name` maps an agent id to a label, for tie-breaking by name.
pub fn summarize(
    skills: &[InstalledSkill],
    facts: &[SkillFacts],
    problems: usize,
    detected_agents: usize,
    display_name: impl Fn(AgentId) -> &'static str,
) -> Summary {
    let mut per_agent: BTreeMap<AgentId, usize> = BTreeMap::new();
    let mut per_source: BTreeMap<String, usize> = BTreeMap::new();
    let mut s = Summary {
        total: skills.len(),
        problems,
        detected_agents,
        ..Summary::default()
    };
    let mut recent: Vec<Recent> = Vec::new();

    for (skill, f) in skills.iter().zip(facts) {
        let agents: HashSet<AgentId> = skill
            .installs
            .iter()
            .map(|i| i.agent)
            .filter(|a| *a != "universal")
            .collect();
        if agents.is_empty() {
            s.unlinked += 1;
        }
        for a in agents {
            *per_agent.entry(a).or_default() += 1;
        }
        s.disk_bytes = s.disk_bytes.saturating_add(f.disk_bytes);
        s.folders += f.copies;

        match &f.source {
            Some(src) => {
                s.managed += 1;
                *per_source.entry(src.clone()).or_default() += 1;
                if let Some(updated) = &f.updated_at {
                    recent.push(Recent {
                        name: skill.meta.display_name(),
                        source: src.clone(),
                        updated_at: updated.clone(),
                    });
                }
            }
            None => s.unmanaged += 1,
        }
    }

    s.agents_in_use = per_agent.len();
    let mut per_agent: Vec<(AgentId, usize)> = per_agent.into_iter().collect();
    per_agent.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| {
            display_name(a.0)
                .to_lowercase()
                .cmp(&display_name(b.0).to_lowercase())
        })
    });
    s.per_agent = per_agent;

    let mut per_source: Vec<Count> = per_source
        .into_iter()
        .map(|(label, count)| Count { label, count })
        .collect();
    per_source.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    s.per_source = per_source;

    // ISO-8601 timestamps sort chronologically as strings.
    recent.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    recent.truncate(5);
    s.recent = recent;
    s
}

/// Formats a count with thousands separators: `6,802`.
pub fn format_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Formats a byte count: `840 B`, `12.4 KB`, `3.1 MB` (powers of 1024).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Days since 1970-01-01 for today (UTC).
pub fn today() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    i64::try_from(secs / 86_400).unwrap_or(0)
}

/// Parses the date part of an ISO-8601 timestamp into days since 1970-01-01.
pub fn parse_iso_date(iso: &str) -> Option<i64> {
    let date = iso.get(..10)?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || date.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// `today`, `yesterday`, `3 days ago`, `2 weeks ago`, `5 months ago`, `2 years ago`.
pub fn relative_date(iso: &str, today: i64) -> Option<String> {
    let days = (today - parse_iso_date(iso)?).max(0);
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    Some(match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=13 => plural(days, "day"),
        14..=59 => plural(days / 7, "week"),
        60..=729 => plural(days / 30, "month"),
        _ => plural(days / 365, "year"),
    })
}

/// Current UTC time as an ISO-8601 timestamp, e.g. `2026-11-03T04:05:06.000Z`
/// (the format the CLI writes to its lockfile).
pub fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_from_unix(secs)
}

pub fn iso_from_unix(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// Inverse of `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Howard Hinnant's days-from-civil algorithm (proleptic Gregorian, UTC).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Scope;
    use crate::scan::AgentInstall;
    use crate::skill_md::SkillMeta;

    fn skill(name: &str, agents: &[(AgentId, LinkKind)]) -> InstalledSkill {
        InstalledSkill {
            meta: SkillMeta {
                name: name.into(),
                ..SkillMeta::default()
            },
            scope: Scope::Global,
            dir_name: name.into(),
            canonical_path: Some(PathBuf::from(format!("/c/{name}"))),
            installs: agents
                .iter()
                .map(|(a, k)| AgentInstall {
                    agent: a,
                    path: PathBuf::from(format!("/a/{a}/{name}")),
                    kind: *k,
                })
                .collect(),
        }
    }

    fn facts(source: Option<&str>, updated: &str, bytes: u64) -> SkillFacts {
        SkillFacts {
            size_bytes: bytes,
            disk_bytes: bytes,
            copies: 1,
            source: source.map(Into::into),
            installed_at: source.map(|_| updated.to_owned()),
            updated_at: source.map(|_| updated.to_owned()),
        }
    }

    #[test]
    fn summarize_counts_and_ranks() {
        let skills = vec![
            skill(
                "a",
                &[
                    ("claude-code", LinkKind::Symlink),
                    ("codex", LinkKind::Canonical),
                ],
            ),
            skill("b", &[("codex", LinkKind::Canonical)]),
            skill("c", &[]),
        ];
        let f = vec![
            facts(Some("o/r"), "2026-10-01T00:00:00Z", 100),
            facts(Some("o/r"), "2026-10-05T00:00:00Z", 50),
            facts(None, "", 10),
        ];
        let s = summarize(&skills, &f, 2, 4, |id| match id {
            "claude-code" => "Claude Code",
            _ => "Codex",
        });
        assert_eq!((s.total, s.managed, s.unmanaged, s.unlinked), (3, 2, 1, 1));
        assert_eq!((s.problems, s.detected_agents, s.agents_in_use), (2, 4, 2));
        assert_eq!(s.disk_bytes, 160);
        assert_eq!(s.per_agent, vec![("codex", 2), ("claude-code", 1)]);
        assert_eq!(
            s.per_source,
            vec![Count {
                label: "o/r".into(),
                count: 2
            }]
        );
        assert_eq!(s.recent[0].name, "b");
    }

    #[test]
    fn dir_size_counts_files_and_skips_symlinks() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("SKILL.md"), [0u8; 100]).expect("write");
        std::fs::create_dir(dir.path().join("refs")).expect("mkdir");
        std::fs::write(dir.path().join("refs/a.md"), [0u8; 50]).expect("write");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/", dir.path().join("root-link")).expect("symlink");
        assert_eq!(dir_size(dir.path()), 150);
    }

    #[test]
    fn count_format() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(6802), "6,802");
        assert_eq!(format_count(1_234_567), "1,234,567");
    }

    #[test]
    fn bytes_format() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(250 * 1024 * 1024), "250 MB");
    }

    #[test]
    fn iso_round_trip() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_unix(1_793_706_306), "2026-11-03T11:45:06.000Z");
        let iso = now_iso();
        assert!(parse_iso_date(&iso).is_some(), "{iso}");
    }

    #[test]
    fn dates() {
        assert_eq!(parse_iso_date("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_date("2000-03-01"), Some(11_017));
        assert_eq!(parse_iso_date("garbage"), None);
        assert_eq!(parse_iso_date("2026-13-01"), None);
        let today = parse_iso_date("2026-11-03").expect("valid");
        let rel = |d| relative_date(d, today).expect("valid");
        assert_eq!(rel("2026-11-03T08:00:00Z"), "today");
        assert_eq!(rel("2026-11-02T08:00:00Z"), "yesterday");
        assert_eq!(rel("2026-10-29"), "5 days ago");
        assert_eq!(rel("2026-10-13"), "3 weeks ago");
        assert_eq!(rel("2026-06-01"), "5 months ago");
        assert_eq!(rel("2023-01-01"), "3 years ago");
        assert_eq!(rel("2027-01-01"), "today");
    }
}
