//! Skill suggestions, each with the reason it was picked.
//!
//! Signals, strongest first:
//! 1. Topics you already use: other skills in a skills.sh topic where you have
//!    at least one skill installed.
//! 2. Repos you already use: popular skills from the official repos you've
//!    installed from.
//! 3. Widely used official skills.
//!
//! Installed skills are never suggested, and no source gets more than three
//! slots, so one big repo doesn't crowd out the rest.

use std::collections::{HashMap, HashSet};

use crate::directory::{to_skill_slug, Official, Topic, TopicDetail, SITE_BASE};
use crate::search::format_installs;

const MAX_PER_SOURCE: usize = 3;
/// At most this many suggestions for the same reason (one topic, one repo).
const MAX_PER_GROUP: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub name: String,
    pub source: String,
    pub slug: String,
    pub reason: String,
    pub url: String,
    score: f64,
    group: String,
}

/// What's installed: `(source, name)` pairs of skills tracked by skills.sh.
pub struct Installed<'a> {
    pub pairs: &'a HashSet<(String, String)>,
}

impl Installed<'_> {
    fn has(&self, source: &str, name: &str) -> bool {
        let slug = to_skill_slug(name);
        self.pairs
            .iter()
            .any(|(s, n)| s == source && (n == name || to_skill_slug(n) == slug))
    }

    fn sources(&self) -> HashSet<&str> {
        self.pairs.iter().map(|(s, _)| s.as_str()).collect()
    }
}

pub fn recommend(
    installed: &Installed<'_>,
    official: Option<&Official>,
    topics: &[(Topic, TopicDetail)],
    limit: usize,
) -> Vec<Suggestion> {
    let mut best: HashMap<(String, String), Suggestion> = HashMap::new();
    let mut offer =
        |name: &str, source: &str, slug: &str, reason: String, score: f64, group: &str| {
            if installed.has(source, name) {
                return;
            }
            let key = (source.to_owned(), slug.to_owned());
            let s = Suggestion {
                name: name.to_owned(),
                source: source.to_owned(),
                slug: slug.to_owned(),
                reason,
                url: format!("{SITE_BASE}/{source}/{slug}"),
                score,
                group: group.to_owned(),
            };
            match best.get(&key) {
                Some(existing) if existing.score >= score => {}
                _ => {
                    best.insert(key, s);
                }
            }
        };

    // 1. Topics you already use.
    for (topic, detail) in topics {
        let have = detail
            .skills
            .iter()
            .filter(|s| installed.has(&s.source, &s.name))
            .count();
        if have == 0 {
            continue;
        }
        for (rank, s) in detail.skills.iter().enumerate() {
            let slug = s.id.rsplit('/').next().unwrap_or(&s.name);
            offer(
                &s.name,
                &s.source,
                slug,
                format!(
                    "In {}, where you have {have} skill{}",
                    topic.title,
                    if have == 1 { "" } else { "s" }
                ),
                // Earlier entries in a topic are its headline picks.
                30.0 + have as f64 * 2.0 - rank as f64 * 0.1,
                &topic.slug,
            );
        }
    }

    if let Some(official) = official {
        let sources = installed.sources();
        // 2. Repos you already use.
        for owner in &official.owners {
            for repo in owner
                .repos
                .iter()
                .filter(|r| sources.contains(r.repo.as_str()))
            {
                for s in &repo.skills {
                    offer(
                        &s.name,
                        &repo.repo,
                        &to_skill_slug(&s.name),
                        format!("Popular in {}, which you already use", repo.repo),
                        20.0 + (s.installs as f64 + 1.0).log10(),
                        &repo.repo,
                    );
                }
            }
        }
        // 3. Widely used official skills.
        for owner in &official.owners {
            for repo in &owner.repos {
                for s in &repo.skills {
                    let installs = format_installs(s.installs);
                    if installs.is_empty() {
                        continue;
                    }
                    offer(
                        &s.name,
                        &repo.repo,
                        &to_skill_slug(&s.name),
                        format!("Official skill from {}, {installs}", owner.owner),
                        (s.installs as f64 + 1.0).log10(),
                        "popular",
                    );
                }
            }
        }
    }

    let mut all: Vec<Suggestion> = best.into_values().collect();
    all.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut per_source: HashMap<String, usize> = HashMap::new();
    let mut per_group: HashMap<String, usize> = HashMap::new();
    all.into_iter()
        .filter(|s| {
            let src = per_source.get(&s.source).copied().unwrap_or(0);
            let grp = per_group.get(&s.group).copied().unwrap_or(0);
            if src >= MAX_PER_SOURCE || grp >= MAX_PER_GROUP {
                return false;
            }
            per_source.insert(s.source.clone(), src + 1);
            per_group.insert(s.group.clone(), grp + 1);
            true
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::{OfficialOwner, OfficialRepo, OfficialSkill, TopicSkill};

    fn topic_skill(name: &str, source: &str) -> TopicSkill {
        TopicSkill {
            name: name.into(),
            description: String::new(),
            source: source.into(),
            id: format!("{source}/{name}"),
        }
    }

    fn official() -> Official {
        let skill = |name: &str, installs| OfficialSkill {
            name: name.into(),
            installs,
        };
        Official {
            owners: vec![OfficialOwner {
                owner: "anthropics".into(),
                total_installs: 0,
                repos: vec![OfficialRepo {
                    repo: "anthropics/skills".into(),
                    total_installs: 0,
                    skills: vec![
                        skill("frontend-design", 400_000),
                        skill("pdf", 100_000),
                        skill("xlsx", 80_000),
                        skill("docx", 70_000),
                        skill("pptx", 60_000),
                    ],
                }],
            }],
        }
    }

    #[test]
    fn ranks_topics_then_repos_and_skips_installed() {
        let pairs: HashSet<(String, String)> = [
            ("anthropics/skills".to_owned(), "frontend-design".to_owned()),
            ("pbakaus/impeccable".to_owned(), "polish".to_owned()),
        ]
        .into();
        let topics = vec![(
            Topic {
                slug: "design".into(),
                title: "Design & UI".into(),
                description: String::new(),
                skill_count: None,
            },
            TopicDetail {
                title: "Design & UI".into(),
                description: String::new(),
                skills: vec![
                    topic_skill("polish", "pbakaus/impeccable"),
                    topic_skill("critique", "pbakaus/impeccable"),
                ],
            },
        )];
        let recs = recommend(&Installed { pairs: &pairs }, Some(&official()), &topics, 10);
        assert_eq!(recs[0].name, "critique");
        assert_eq!(recs[0].reason, "In Design & UI, where you have 1 skill");
        assert_eq!(
            recs[0].url,
            "https://www.skills.sh/pbakaus/impeccable/critique"
        );
        assert!(recs
            .iter()
            .all(|r| r.name != "frontend-design" && r.name != "polish"));
        // At most three from one source.
        assert_eq!(
            recs.iter()
                .filter(|r| r.source == "anthropics/skills")
                .count(),
            3
        );
        assert_eq!(
            recs[1].reason,
            "Popular in anthropics/skills, which you already use"
        );
    }

    #[test]
    fn falls_back_to_popular_official_skills() {
        let pairs = HashSet::new();
        let recs = recommend(&Installed { pairs: &pairs }, Some(&official()), &[], 2);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].name, "frontend-design");
        assert!(recs[0].reason.starts_with("Official skill from anthropics"));
    }
}
