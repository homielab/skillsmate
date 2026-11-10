//! Online directory pages: Discover (search), Official and Topics.
//!
//! Official and Topics are fetched the first time their page opens and kept for
//! the session; "Reload" fetches again.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use skills_core::directory::{to_skill_slug, Official, Topic, TopicDetail, SITE_BASE};
use skills_core::recommend::{recommend, Installed};
use skills_core::sanitize::sanitize_name;
use skills_core::search::{format_installs, MIN_QUERY_LEN};
use slint::SharedString;

use super::{model, plural, Controller};
use crate::{OfficialRow, RecRow, SearchRow, TopicRow, TopicSkillRow};

const MAX_RECOMMENDATIONS: usize = 12;

/// Delay after the last keystroke before searching.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct ExploreState {
    official: Option<Official>,
    official_loading: bool,
    topics: Option<Vec<Topic>>,
    topics_loading: bool,
    topic_cache: HashMap<String, TopicDetail>,
    /// The topic the user has open; responses for other slugs are dropped.
    open_topic: Option<String>,
    recs_loading: bool,
}

impl ExploreState {
    pub(super) fn has_recommendation_data(&self) -> bool {
        self.official.is_some() || !self.topic_cache.is_empty()
    }
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

impl Controller {
    pub(super) fn search(&self, query: String) {
        let Some(client) = self.search.clone() else {
            return;
        };
        let generation = Arc::clone(&self.search_generation);
        let my_gen = generation.fetch_add(1, Ordering::SeqCst) + 1;
        let weak = self.ui.clone();
        let installed = self.state.borrow().library.installed.clone();

        self.rt.spawn(async move {
            tokio::time::sleep(SEARCH_DEBOUNCE).await;
            if generation.load(Ordering::SeqCst) != my_gen {
                return; // A newer keystroke superseded this search.
            }
            let still_current = {
                let generation = Arc::clone(&generation);
                move || generation.load(Ordering::SeqCst) == my_gen
            };

            let empty = query.trim().chars().count() < MIN_QUERY_LEN;
            let _ = weak.upgrade_in_event_loop({
                let still_current = still_current.clone();
                move |ui| {
                    if still_current() {
                        ui.set_searching(!empty);
                        ui.set_search_status(SharedString::default());
                        if empty {
                            ui.set_search_results(model(Vec::new()));
                        }
                    }
                }
            });
            if empty {
                return;
            }

            let result = client.search(&query, None).await;
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if !still_current() {
                    return;
                }
                ui.set_searching(false);
                match result {
                    Ok(hits) => {
                        if hits.is_empty() {
                            ui.set_search_status(
                                format!("No skills match \"{}\".", query.trim()).into(),
                            );
                        }
                        let rows: Vec<SearchRow> = hits
                            .iter()
                            .map(|h| {
                                // The id is `{source}/{slug}`.
                                let slug = h
                                    .slug
                                    .strip_prefix(&format!("{}/", h.source))
                                    .unwrap_or(&h.name)
                                    .to_owned();
                                SearchRow {
                                    name: h.name.as_str().into(),
                                    source: h.source.as_str().into(),
                                    installed: is_installed(&installed, &h.source, &h.name),
                                    installable: installable(&h.source),
                                    slug: slug.into(),
                                    installs: format_installs(h.installs).into(),
                                    url: h.page_url(SITE_BASE).into(),
                                }
                            })
                            .collect();
                        ui.set_search_results(model(rows));
                    }
                    Err(e) => {
                        ui.set_search_results(model(Vec::new()));
                        ui.set_search_status(format!("Search failed: {e}").into());
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------ official

    pub(super) fn load_official(&self, force: bool) {
        let Some(client) = self.directory.clone() else {
            return;
        };
        {
            let mut st = self.state.borrow_mut();
            if st.explore.official_loading || (!force && st.explore.official.is_some()) {
                return;
            }
            st.explore.official_loading = true;
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_official_loading(true);
            ui.set_official_status("Loading from skills.sh…".into());
        }
        let state = std::rc::Rc::clone(&self.state);
        let this = self.clone();
        self.run(async move { client.official().await }, move |ui, result| {
            {
                let mut st = state.borrow_mut();
                st.explore.official_loading = false;
                match result {
                    Ok(official) => st.explore.official = Some(official),
                    Err(e) => {
                        ui.set_official_status(
                            format!("Couldn't load the official list: {e}").into(),
                        );
                    }
                }
            }
            ui.set_official_loading(false);
            this.filter_official(&ui.get_official_filter());
        });
    }

    pub(super) fn filter_official(&self, filter: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let st = self.state.borrow();
        let Some(official) = &st.explore.official else {
            return;
        };
        let needle = filter.trim().to_lowercase();
        let installed = &st.library.installed;
        let rows: Vec<OfficialRow> = official
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
                    owner: o.owner.as_str().into(),
                    stats: stats.into(),
                    top: format!("Popular: {}", o.top_skills(4).join(", ")).into(),
                    installed: if mine == 0 {
                        "".into()
                    } else {
                        format!("{mine} installed").into()
                    },
                    url: format!("{SITE_BASE}/{}", o.owner).into(),
                }
            })
            .collect();
        let total_skills: usize = official.owners.iter().map(|o| o.skill_count()).sum();
        ui.set_official_status(
            format!(
                "{} from {} companies that build the technology",
                plural(total_skills, "skill", "skills"),
                official.owners.len()
            )
            .into(),
        );
        ui.set_official(model(rows));
    }

    // -------------------------------------------------------------- topics

    pub(super) fn load_topics(&self, force: bool) {
        let Some(client) = self.directory.clone() else {
            return;
        };
        {
            let mut st = self.state.borrow_mut();
            if st.explore.topics_loading || (!force && st.explore.topics.is_some()) {
                return;
            }
            st.explore.topics_loading = true;
            if force {
                st.explore.topic_cache.clear();
            }
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_topics_loading(true);
            ui.set_topics_status("Loading from skills.sh…".into());
        }
        let state = std::rc::Rc::clone(&self.state);
        let this = self.clone();
        self.run(async move { client.topics().await }, move |ui, result| {
            let open = {
                let mut st = state.borrow_mut();
                st.explore.topics_loading = false;
                match result {
                    Ok(topics) => {
                        ui.set_topics_status("Curated collections of skills by domain".into());
                        let rows: Vec<TopicRow> = topics
                            .iter()
                            .map(|t| TopicRow {
                                slug: t.slug.as_str().into(),
                                title: t.title.as_str().into(),
                                description: t.description.as_str().into(),
                                count: t
                                    .skill_count
                                    .map(|n| plural(n as usize, "skill", "skills"))
                                    .unwrap_or_default()
                                    .into(),
                            })
                            .collect();
                        ui.set_topics(model(rows));
                        st.explore.topics = Some(topics);
                    }
                    Err(e) => {
                        ui.set_topics_status(format!("Couldn't load topics: {e}").into());
                    }
                }
                st.explore.open_topic.clone()
            };
            ui.set_topics_loading(false);
            // A reload while a topic is open refreshes it too.
            if let Some(slug) = open {
                this.open_topic(slug);
            }
        });
    }

    pub(super) fn open_topic(&self, slug: String) {
        let Some(ui) = self.ui.upgrade() else { return };
        let cached = {
            let mut st = self.state.borrow_mut();
            st.explore.open_topic = Some(slug.clone());
            st.explore.topic_cache.get(&slug).cloned()
        };
        ui.set_topic_slug(slug.as_str().into());
        if let Some(detail) = cached {
            self.show_topic(&detail);
            return;
        }
        let title = self
            .state
            .borrow()
            .explore
            .topics
            .as_ref()
            .and_then(|ts| ts.iter().find(|t| t.slug == slug))
            .map(|t| t.title.clone())
            .unwrap_or_else(|| slug.clone());
        ui.set_topic_title(title.into());
        ui.set_topic_description(SharedString::default());
        ui.set_topic_skills(model(Vec::new()));
        ui.set_topics_loading(true);
        ui.set_topics_status("Loading…".into());

        let Some(client) = self.directory.clone() else {
            return;
        };
        let state = std::rc::Rc::clone(&self.state);
        let this = self.clone();
        let wanted = slug.clone();
        self.run(
            async move { client.topic(&wanted).await },
            move |ui, result| {
                let current = state.borrow().explore.open_topic.clone();
                if current.as_deref() != Some(slug.as_str()) {
                    return; // The user moved on.
                }
                ui.set_topics_loading(false);
                match result {
                    Ok(detail) => {
                        state
                            .borrow_mut()
                            .explore
                            .topic_cache
                            .insert(slug, detail.clone());
                        this.show_topic(&detail);
                    }
                    Err(e) => ui.set_topics_status(format!("Couldn't load this topic: {e}").into()),
                }
            },
        );
    }

    fn show_topic(&self, detail: &TopicDetail) {
        let Some(ui) = self.ui.upgrade() else { return };
        let st = self.state.borrow();
        let installed = &st.library.installed;
        let rows: Vec<TopicSkillRow> = detail
            .skills
            .iter()
            .map(|s| TopicSkillRow {
                name: s.name.as_str().into(),
                description: s.description.as_str().into(),
                source: s.source.as_str().into(),
                slug: s.id.rsplit('/').next().unwrap_or(&s.name).into(),
                installed: is_installed(installed, &s.source, &s.name),
                installable: installable(&s.source),
                url: s.url().into(),
            })
            .collect();
        let mine = rows.iter().filter(|r| r.installed).count();
        let mut status = plural(rows.len(), "skill", "skills");
        if mine > 0 {
            status.push_str(&format!(" · {mine} installed"));
        }
        ui.set_topic_title(detail.title.as_str().into());
        ui.set_topic_description(detail.description.as_str().into());
        ui.set_topics_status(status.into());
        ui.set_topic_skills(model(rows));
    }

    pub(super) fn close_topic(&self) {
        self.state.borrow_mut().explore.open_topic = None;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_topic_slug(SharedString::default());
            ui.set_topics_status("Curated collections of skills by domain".into());
        }
    }

    // ------------------------------------------------------ recommendations

    /// Fetches whatever recommendations need that isn't cached yet (official
    /// list, topics and their skills), then shows them.
    pub(super) fn load_recommendations(&self) {
        let Some(client) = self.directory.clone() else {
            return;
        };
        let (need_official, cached_topics, cached_slugs) = {
            let mut st = self.state.borrow_mut();
            let ex = &mut st.explore;
            if ex.recs_loading {
                return;
            }
            let have_details = ex
                .topics
                .as_ref()
                .is_some_and(|ts| ts.iter().all(|t| ex.topic_cache.contains_key(&t.slug)));
            if ex.official.is_some() && have_details {
                drop(st);
                self.show_recommendations();
                return;
            }
            ex.recs_loading = true;
            (
                ex.official.is_none(),
                ex.topics.clone(),
                ex.topic_cache.keys().cloned().collect::<HashSet<_>>(),
            )
        };
        if let Some(ui) = self.ui.upgrade() {
            ui.set_recs_loading(true);
            ui.set_recs_status("Looking at what you have…".into());
        }
        let state = std::rc::Rc::clone(&self.state);
        let this = self.clone();
        self.run(
            async move {
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
                (official, fetched_topics, details)
            },
            move |_, (official, topics, details)| {
                {
                    let mut st = state.borrow_mut();
                    let ex = &mut st.explore;
                    ex.recs_loading = false;
                    if official.is_some() {
                        ex.official = official;
                    }
                    if topics.is_some() {
                        ex.topics = topics;
                    }
                    ex.topic_cache.extend(details);
                }
                this.show_recommendations();
            },
        );
    }

    /// Recomputes recommendations from cached data (e.g. after an install).
    pub(super) fn show_recommendations(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let st = self.state.borrow();
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
        let rows: Vec<RecRow> = recs
            .into_iter()
            .map(|r| RecRow {
                name: r.name.into(),
                source: r.source.into(),
                slug: r.slug.into(),
                reason: r.reason.into(),
                url: r.url.into(),
            })
            .collect();
        ui.set_recs(model(rows));
        ui.set_recs_status(status.into());
        ui.set_recs_loading(ex.recs_loading);
    }
}
