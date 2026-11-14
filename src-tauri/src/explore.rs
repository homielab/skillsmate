//! Online directory pages: Discover (search and recommendations), Official
//! and Topics.
//!
//! Official and Topics are fetched the first time they're asked for and kept
//! for the session; `force` fetches again.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use skills_core::directory::{to_skill_slug, Official, Topic, TopicDetail, SITE_BASE};
use skills_core::recommend::{recommend, Installed};
use skills_core::sanitize::sanitize_name;
use skills_core::search::{format_installs, MIN_QUERY_LEN};

use crate::state::{plural, AppState, State};

const MAX_RECOMMENDATIONS: usize = 12;

#[derive(Default)]
pub struct ExploreState {
    official: Option<Official>,
    topics: Option<Vec<Topic>>,
    topic_cache: HashMap<String, TopicDetail>,
}

/// Whether `(source, name)` is installed, matching by name or slug.
fn is_installed(installed: &HashSet<(String, String)>, source: &str, name: &str) -> bool {
    let slug = to_skill_slug(name);
    installed
        .iter()
        .any(|(s, n)| s == source && (n == name || to_skill_slug(n) == slug))
}

/// Only GitHub `owner/repo` sources can be downloaded from skills.sh.
fn installable(source: &str) -> bool {
    source.matches('/').count() == 1
}

/// A skill on skills.sh, as listed in search results, topics and
/// recommendations.
#[derive(Serialize)]
pub struct SkillCard {
    name: String,
    /// Description, or why it's recommended. May be empty.
    description: String,
    source: String,
    slug: String,
    /// "12.3K installs", or "".
    installs: String,
    url: String,
    installed: bool,
    installable: bool,
}

#[derive(Serialize)]
pub struct CardList {
    rows: Vec<SkillCard>,
    status: String,
}

// ----------------------------------------------------------------- search

/// Searches skills.sh. The frontend debounces and drops stale responses.
#[tauri::command]
pub async fn search(state: tauri::State<'_, AppState>, query: String) -> Result<CardList, String> {
    let query = query.trim().to_owned();
    if query.chars().count() < MIN_QUERY_LEN {
        return Ok(CardList {
            rows: Vec::new(),
            status: String::new(),
        });
    }
    let client = state
        .search
        .clone()
        .ok_or("Online features are unavailable.")?;
    let hits = client
        .search(&query, None)
        .await
        .map_err(|e| format!("Search failed: {e}"))?;
    let st = state.lock();
    let installed = &st.library.installed;
    let rows: Vec<SkillCard> = hits
        .iter()
        .map(|h| SkillCard {
            name: h.name.clone(),
            description: String::new(),
            source: h.source.clone(),
            // The id is `{source}/{slug}`.
            slug: h
                .slug
                .strip_prefix(&format!("{}/", h.source))
                .unwrap_or(&h.name)
                .to_owned(),
            installs: format_installs(h.installs),
            url: h.page_url(SITE_BASE),
            installed: is_installed(installed, &h.source, &h.name),
            installable: installable(&h.source),
        })
        .collect();
    let status = if rows.is_empty() {
        format!("No skills match \"{query}\".")
    } else {
        plural(rows.len(), "result", "results")
    };
    Ok(CardList { rows, status })
}

// --------------------------------------------------------------- official

#[derive(Serialize)]
struct OfficialRow {
    owner: String,
    stats: String,
    /// Most installed skills, for the row's second line.
    top: Vec<String>,
    /// "3 installed", or "".
    installed: String,
    url: String,
}

#[derive(Serialize)]
pub struct OfficialView {
    rows: Vec<OfficialRow>,
    status: String,
}

/// Companies that publish first-party skills, filtered by company, repo or
/// skill name.
#[tauri::command]
pub async fn official(
    state: tauri::State<'_, AppState>,
    force: bool,
    filter: String,
) -> Result<OfficialView, String> {
    if force || state.lock().explore.official.is_none() {
        let client = state.directory()?;
        let official = client
            .official()
            .await
            .map_err(|e| format!("Couldn't load the official list: {e}"))?;
        state.lock().explore.official = Some(official);
    }
    let st = state.lock();
    let Some(official) = &st.explore.official else {
        return Err("Couldn't load the official list.".into());
    };
    let needle = filter.trim().to_lowercase();
    let installed = &st.library.installed;
    let rows = official
        .owners
        .iter()
        .filter(|o| {
            needle.is_empty()
                || o.owner.to_lowercase().contains(&needle)
                || o.repos.iter().any(|r| {
                    r.repo.to_lowercase().contains(&needle)
                        || r.skills
                            .iter()
                            .any(|s| s.name.to_lowercase().contains(&needle))
                })
        })
        .map(|o| {
            let prefix = format!("{}/", o.owner);
            // Installed skills are recorded under both their frontmatter
            // name and folder name; normalising collapses the pair.
            let mine = installed
                .iter()
                .filter(|(src, _)| src.starts_with(&prefix) || *src == o.owner)
                .map(|(src, name)| (src, sanitize_name(name)))
                .collect::<HashSet<_>>()
                .len();
            let installs = format_installs(o.total_installs);
            let mut stats = format!(
                "{} · {}",
                plural(o.repos.len(), "repo", "repos"),
                plural(o.skill_count(), "skill", "skills")
            );
            if !installs.is_empty() {
                stats.push_str(&format!(" · {installs}"));
            }
            OfficialRow {
                owner: o.owner.clone(),
                stats,
                top: o.top_skills(4).into_iter().map(str::to_owned).collect(),
                installed: if mine == 0 {
                    String::new()
                } else {
                    format!("{mine} installed")
                },
                url: format!("{SITE_BASE}/{}", o.owner),
            }
        })
        .collect();
    let total_skills: usize = official.owners.iter().map(|o| o.skill_count()).sum();
    Ok(OfficialView {
        rows,
        status: format!(
            "{} from {} companies that build the technology",
            plural(total_skills, "skill", "skills"),
            official.owners.len()
        ),
    })
}

// ----------------------------------------------------------------- topics

#[derive(Serialize)]
pub struct TopicRow {
    slug: String,
    title: String,
    description: String,
    /// "24 skills", or "".
    count: String,
}

#[tauri::command]
pub async fn topics(
    state: tauri::State<'_, AppState>,
    force: bool,
) -> Result<Vec<TopicRow>, String> {
    if force || state.lock().explore.topics.is_none() {
        let client = state.directory()?;
        let topics = client
            .topics()
            .await
            .map_err(|e| format!("Couldn't load topics: {e}"))?;
        let mut st = state.lock();
        if force {
            st.explore.topic_cache.clear();
        }
        st.explore.topics = Some(topics);
    }
    let st = state.lock();
    Ok(st
        .explore
        .topics
        .iter()
        .flatten()
        .map(|t| TopicRow {
            slug: t.slug.clone(),
            title: t.title.clone(),
            description: t.description.clone(),
            count: t
                .skill_count
                .map(|n| plural(n as usize, "skill", "skills"))
                .unwrap_or_default(),
        })
        .collect())
}

#[derive(Serialize)]
pub struct TopicView {
    title: String,
    description: String,
    status: String,
    rows: Vec<SkillCard>,
}

/// The skills in one topic.
#[tauri::command]
pub async fn topic(state: tauri::State<'_, AppState>, slug: String) -> Result<TopicView, String> {
    let cached = state.lock().explore.topic_cache.get(&slug).cloned();
    let detail = match cached {
        Some(detail) => detail,
        None => {
            let client = state.directory()?;
            let detail = client
                .topic(&slug)
                .await
                .map_err(|e| format!("Couldn't load this topic: {e}"))?;
            state
                .lock()
                .explore
                .topic_cache
                .insert(slug, detail.clone());
            detail
        }
    };
    let st = state.lock();
    let installed = &st.library.installed;
    let rows: Vec<SkillCard> = detail
        .skills
        .iter()
        .map(|s| SkillCard {
            name: s.name.clone(),
            description: s.description.clone(),
            source: s.source.clone(),
            slug: s.id.rsplit('/').next().unwrap_or(&s.name).to_owned(),
            installs: String::new(),
            url: s.url(),
            installed: is_installed(installed, &s.source, &s.name),
            installable: installable(&s.source),
        })
        .collect();
    let mine = rows.iter().filter(|r| r.installed).count();
    let mut status = plural(rows.len(), "skill", "skills");
    if mine > 0 {
        status.push_str(&format!(" · {mine} installed"));
    }
    Ok(TopicView {
        title: detail.title,
        description: detail.description,
        status,
        rows,
    })
}

// -------------------------------------------------------- recommendations

/// Suggestions from the topics and repos the user already uses, and popular
/// official skills. With `fetch`, first downloads whatever that needs and
/// isn't cached yet; without, recomputes from what's cached (after an
/// install, say).
#[tauri::command]
pub async fn recommendations(
    state: tauri::State<'_, AppState>,
    fetch: bool,
) -> Result<CardList, String> {
    if fetch {
        if let Some(client) = state.directory.clone() {
            let (need_official, cached_topics, cached_slugs) = {
                let st = state.lock();
                let ex = &st.explore;
                (
                    ex.official.is_none(),
                    ex.topics.clone(),
                    ex.topic_cache.keys().cloned().collect::<HashSet<_>>(),
                )
            };
            let official = if need_official {
                client.official().await.ok()
            } else {
                None
            };
            let fetched_topics = if cached_topics.is_none() {
                client.topics().await.ok()
            } else {
                None
            };
            let mut details = Vec::new();
            for t in fetched_topics
                .as_ref()
                .or(cached_topics.as_ref())
                .into_iter()
                .flatten()
            {
                if !cached_slugs.contains(&t.slug) {
                    if let Ok(d) = client.topic(&t.slug).await {
                        details.push((t.slug.clone(), d));
                    }
                }
            }
            let mut st = state.lock();
            let ex = &mut st.explore;
            if official.is_some() {
                ex.official = official;
            }
            if fetched_topics.is_some() {
                ex.topics = fetched_topics;
            }
            ex.topic_cache.extend(details);
        }
    }
    Ok(recommend_from_cache(&state.lock()))
}

fn recommend_from_cache(st: &State) -> CardList {
    let ex = &st.explore;
    let topics: Vec<(Topic, TopicDetail)> = ex
        .topics
        .iter()
        .flatten()
        .filter_map(|t| Some((t.clone(), ex.topic_cache.get(&t.slug)?.clone())))
        .collect();
    let recs = recommend(
        &Installed {
            pairs: &st.library.installed,
        },
        ex.official.as_ref(),
        &topics,
        MAX_RECOMMENDATIONS,
    );
    let status = if recs.is_empty() && ex.official.is_none() {
        "Couldn't reach skills.sh"
    } else if st.library.installed.is_empty() {
        "Popular official skills to start with"
    } else {
        "Based on the skills you already use"
    };
    CardList {
        rows: recs
            .into_iter()
            .map(|r| SkillCard {
                name: r.name,
                description: r.reason,
                installs: String::new(),
                url: r.url,
                installed: false,
                installable: installable(&r.source),
                source: r.source,
                slug: r.slug,
            })
            .collect(),
        status: status.to_owned(),
    }
}
