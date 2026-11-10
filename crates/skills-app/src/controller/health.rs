//! The Health page: local findings, online update checks, and one-click fixes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use skills_core::directory::{to_skill_slug, DirectoryClient};
use skills_core::files::{diff, read_skill_files, FileDiff, FileMap};
use skills_core::health::{round_k, Fix, Severity};
use skills_core::manage::{compare, load_baselines, UpdateState};
use skills_core::Env;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::library::display_name;
use super::{model, plural, to_i32, Controller};
use crate::{FindingRow, UpdateRow};

const CONCURRENCY: usize = 4;
pub(super) const MAX_DIFF_LINES: usize = 400;

/// The latest update check for one skill.
pub(super) struct UpdateInfo {
    pub state: UpdateState,
    pub diff: FileDiff,
    /// The published files, kept so "Update" doesn't download again.
    pub remote: FileMap,
    pub source: String,
}

impl UpdateInfo {
    /// Short label for the Installed list, "" when up to date.
    pub fn badge(&self) -> &'static str {
        match self.state {
            UpdateState::UpToDate => "",
            s => s.label(),
        }
    }

    pub fn kind(&self) -> i32 {
        match self.state {
            UpdateState::UpToDate => 0,
            UpdateState::UpdateAvailable => 1,
            _ => 4,
        }
    }
}

#[derive(Default)]
pub(super) struct UpdatesState {
    pub results: HashMap<String, UpdateInfo>,
    pub running: bool,
    pub checked: usize,
    pub unavailable: usize,
}

struct Target {
    name: String,
    source: String,
    slugs: Vec<String>,
    path: PathBuf,
}

enum Checked {
    Done(UpdateInfo),
    Unavailable,
}

pub(super) fn finding_kind(s: Severity) -> i32 {
    match s {
        Severity::Problem => 2,
        Severity::Warning => 1,
        Severity::Tip => 4,
    }
}

pub(super) fn fix_label(f: Option<Fix>) -> &'static str {
    match f {
        Some(Fix::RemoveBrokenLink) => "Remove link",
        Some(Fix::RelinkCopy) => "Use main copy",
        None => "",
    }
}

impl Controller {
    /// Fills the Health page and its nav badge from the latest scan and update
    /// check.
    pub(super) fn show_health(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let st = self.state.borrow();
        let lib = &st.library;
        let h = &lib.health;

        let rows: Vec<FindingRow> = h
            .findings
            .iter()
            .enumerate()
            .map(|(i, f)| FindingRow {
                title: f.title.as_str().into(),
                detail: f.detail.as_str().into(),
                skill: f
                    .skill
                    .as_deref()
                    .map(|n| {
                        lib.skills
                            .iter()
                            .find(|s| s.meta.name == n)
                            .map_or_else(|| n.to_owned(), |s| s.meta.display_name())
                    })
                    .unwrap_or_default()
                    .into(),
                key: f.skill.clone().unwrap_or_default().into(),
                kind: finding_kind(f.severity),
                fix_label: fix_label(f.fix).into(),
                index: to_i32(i),
            })
            .collect();

        let (problems, warnings, tips) = (
            h.count(Severity::Problem),
            h.count(Severity::Warning),
            h.count(Severity::Tip),
        );
        let updates = &st.updates;
        let available = updates
            .results
            .values()
            .filter(|u| u.state == UpdateState::UpdateAvailable)
            .count();

        let mut parts = Vec::new();
        if problems > 0 {
            parts.push(plural(problems, "problem", "problems"));
        }
        if warnings > 0 {
            parts.push(plural(warnings, "warning", "warnings"));
        }
        if available > 0 {
            parts.push(plural(available, "update", "updates"));
        }
        if tips > 0 {
            parts.push(plural(tips, "tip", "tips"));
        }
        let summary = if parts.is_empty() {
            "Everything looks good".to_owned()
        } else {
            parts.join(" · ")
        };

        let (startup, startup_note) = match h.startup_by_agent.first() {
            Some((agent, tokens, n)) => (
                format!("~{} tokens", round_k(*tokens)),
                format!("{}, {}", display_name(agent), plural(*n, "skill", "skills")),
            ),
            None => ("0".into(), "No agent sees any skills".into()),
        };

        ui.set_health_summary(summary.into());
        ui.set_health_problems(problems.to_string().into());
        ui.set_health_warnings(warnings.to_string().into());
        ui.set_health_tips(tips.to_string().into());
        ui.set_health_startup(startup.into());
        ui.set_health_startup_note(startup_note.into());
        ui.set_findings(model(rows));
        ui.set_broken_links(to_i32(broken_links(&lib.health.findings).len()));
        ui.set_health_badge(to_i32(problems + warnings + available));

        // Updates: anything not up to date, update-available first.
        let mut list: Vec<(&String, &UpdateInfo)> = updates
            .results
            .iter()
            .filter(|(_, u)| u.state != UpdateState::UpToDate)
            .collect();
        list.sort_by(|a, b| {
            (a.1.state != UpdateState::UpdateAvailable)
                .cmp(&(b.1.state != UpdateState::UpdateAvailable))
                .then_with(|| a.0.cmp(b.0))
        });
        let update_rows: Vec<UpdateRow> = list
            .into_iter()
            .map(|(key, u)| UpdateRow {
                name: lib
                    .skills
                    .iter()
                    .find(|s| &s.meta.name == key)
                    .map_or_else(|| key.clone(), |s| s.meta.display_name())
                    .into(),
                key: key.as_str().into(),
                source: u.source.as_str().into(),
                label: u.state.label().into(),
                kind: u.kind(),
                summary: u.diff.summary().into(),
                can_update: true,
            })
            .collect();
        let status = if updates.running {
            "Checking…".to_owned()
        } else if updates.checked == 0 {
            String::new()
        } else {
            let current = updates
                .results
                .values()
                .filter(|u| u.state == UpdateState::UpToDate)
                .count();
            let mut s = format!("{current} up to date");
            if available > 0 {
                s.push_str(&format!(" · {}", plural(available, "update", "updates")));
            }
            let count = |state: UpdateState| {
                updates
                    .results
                    .values()
                    .filter(|u| u.state == state)
                    .count()
            };
            let edited = count(UpdateState::EditedLocally) + count(UpdateState::Diverged);
            if edited > 0 {
                s.push_str(&format!(" · {edited} edited locally"));
            }
            let differs = count(UpdateState::Differs);
            if differs > 0 {
                s.push_str(&format!(" · {differs} differ from skills.sh"));
            }
            if updates.unavailable > 0 {
                s.push_str(&format!(" · {} not on skills.sh", updates.unavailable));
            }
            s
        };
        // Keep the "Check" prompt visible until a check has run; after that,
        // an empty list with a status line means everything is current.
        ui.set_updates(model(update_rows));
        ui.set_updates_status(status.into());
        ui.set_updates_checking(updates.running);
    }

    pub(super) fn check_updates(&self) {
        let Some(client) = self.directory.clone() else {
            self.show_error("Online features are unavailable.");
            return;
        };
        let targets: Vec<Target> = {
            let mut st = self.state.borrow_mut();
            if st.updates.running {
                return;
            }
            st.updates.running = true;
            let lib = &st.library;
            lib.skills
                .iter()
                .zip(&lib.facts)
                .filter_map(|(s, f)| {
                    let source = f.source.clone()?;
                    // Only GitHub owner/repo sources have downloadable snapshots.
                    if source.matches('/').count() != 1 {
                        return None;
                    }
                    let mut slugs = vec![s.dir_name.clone()];
                    let alt = to_skill_slug(&s.meta.name);
                    if !slugs.contains(&alt) {
                        slugs.push(alt);
                    }
                    Some(Target {
                        name: s.meta.name.clone(),
                        source,
                        slugs,
                        path: s.primary_path().to_path_buf(),
                    })
                })
                .collect()
        };
        self.show_health();
        if targets.is_empty() {
            self.state.borrow_mut().updates.running = false;
            self.show_health();
            self.show_toast("No skills from skills.sh to check.", false, false, false);
            return;
        }

        let env = Arc::clone(&self.env);
        let state = std::rc::Rc::clone(&self.state);
        let this = self.clone();
        self.run(check_all(client, env, targets), move |_, results| {
            {
                let mut st = state.borrow_mut();
                let u = &mut st.updates;
                u.running = false;
                u.checked = results.len();
                u.unavailable = 0;
                for (name, r) in results {
                    match r {
                        Checked::Done(info) => {
                            u.results.insert(name, info);
                        }
                        Checked::Unavailable => {
                            u.unavailable += 1;
                            u.results.remove(&name);
                        }
                    }
                }
                let available = u
                    .results
                    .values()
                    .filter(|i| i.state == UpdateState::UpdateAvailable)
                    .count();
                let attention = &mut st.library.overview.attention;
                attention.retain(|(t, _)| {
                    !t.contains("update available") && !t.contains("updates available")
                });
                if available > 0 {
                    attention.insert(
                        0,
                        (
                            format!(
                                "{} available. See Health.",
                                plural(available, "update", "updates")
                            ),
                            1,
                        ),
                    );
                }
            }
            if let Some(ui) = this.ui.upgrade() {
                let st = state.borrow();
                ui.set_overview(st.library.overview.to_ui());
            }
            this.show_health();
            this.filter_installed(
                &this
                    .ui
                    .upgrade()
                    .map(|u| u.get_installed_filter())
                    .unwrap_or_default(),
            );
            this.refresh_detail();
        });
    }

    /// Runs a finding's one-click fix, after confirmation.
    pub(super) fn fix(&self, index: i32) {
        let finding = {
            let st = self.state.borrow();
            usize::try_from(index)
                .ok()
                .and_then(|i| st.library.health.findings.get(i).cloned())
        };
        let Some(f) = finding else { return };
        let (Some(fix), Some(path)) = (f.fix, f.path.clone()) else {
            return;
        };
        let (title, message, confirm) = match fix {
            Fix::RemoveBrokenLink => (
                "Remove broken link?".to_owned(),
                format!(
                    "{} points to a skill that no longer exists. It will be removed; you can undo this.",
                    path.display()
                ),
                "Remove link",
            ),
            Fix::RelinkCopy => (
                "Use the main copy?".to_owned(),
                format!(
                    "{} will be replaced with a link to the main copy, so this agent always uses the same version as the others. The old copy is kept in a backup you can restore.",
                    path.display()
                ),
                "Use main copy",
            ),
        };
        self.ask(
            &title,
            &message,
            confirm,
            fix == Fix::RemoveBrokenLink,
            super::actions::Pending::Fix(f),
        );
    }
}

async fn check_all(
    client: DirectoryClient,
    env: Arc<Env>,
    targets: Vec<Target>,
) -> Vec<(String, Checked)> {
    let baselines = Arc::new(
        tokio::task::spawn_blocking(move || load_baselines(&env))
            .await
            .unwrap_or_default(),
    );
    let permits = Arc::new(Semaphore::new(CONCURRENCY));
    let mut set = JoinSet::new();
    for t in targets {
        let client = client.clone();
        let permits = Arc::clone(&permits);
        let baselines = Arc::clone(&baselines);
        set.spawn(async move {
            let _permit = permits.acquire().await;
            let mut remote = None;
            for slug in &t.slugs {
                if let Ok(Some(files)) = client.download(&t.source, slug).await {
                    remote = Some(files);
                    break;
                }
            }
            let Some(remote) = remote else {
                return (t.name, Checked::Unavailable);
            };
            let path = t.path.clone();
            let local = tokio::task::spawn_blocking(move || read_skill_files(&path))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            let state = compare(&local, &remote, baselines.get(&t.name).map(String::as_str));
            let diff = diff(&local, &remote, MAX_DIFF_LINES);
            (
                t.name,
                Checked::Done(UpdateInfo {
                    state,
                    diff,
                    remote,
                    source: t.source,
                }),
            )
        });
    }
    let mut out = Vec::new();
    while let Some(Ok(r)) = set.join_next().await {
        out.push(r);
    }
    out
}

impl Controller {
    /// Asks to remove every broken link the last scan found, in one step.
    pub(super) fn fix_all_links(&self) {
        let paths = broken_links(&self.state.borrow().library.health.findings);
        if paths.is_empty() {
            return;
        }
        let shown: Vec<String> = paths
            .iter()
            .take(3)
            .map(|p| skills_core::health::pretty_path(p, self.env.home()))
            .collect();
        let more = match paths.len().saturating_sub(shown.len()) {
            0 => String::new(),
            n => format!("\nand {n} more"),
        };
        let n = paths.len();
        self.ask(
            &format!("Remove {n} broken links?"),
            &format!(
                "These links point to skills that no longer exist:\n{}{more}\n\nThey'll be removed together; one Undo puts them all back.",
                shown.join("\n")
            ),
            "Remove links",
            true,
            super::actions::Pending::FixAllLinks(paths),
        );
    }
}

/// Paths of the broken links among `findings`, in finding order.
fn broken_links(findings: &[skills_core::health::Finding]) -> Vec<std::path::PathBuf> {
    findings
        .iter()
        .filter(|f| f.fix == Some(Fix::RemoveBrokenLink))
        .filter_map(|f| f.path.clone())
        .collect()
}
