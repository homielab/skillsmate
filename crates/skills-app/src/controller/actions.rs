//! Actions that change files: install, update, remove and fixes, each behind a
//! confirmation dialog and followed by a toast with Undo. Also the app's own
//! update check.

use std::sync::Arc;
use std::time::Duration;

use skills_core::agents::{detected_agents, Agent};
use skills_core::app_update::{is_newer, latest_release};
use skills_core::files::{script_files, FileMap};
use skills_core::health::{Finding, Fix};
use skills_core::manage::{self, Outcome, UpdateState};
use skills_core::skill_md;

use super::library::display_name;
use super::{plural, release_repo, Controller};

/// How long a result toast stays up.
const TOAST_SECONDS: u64 = 8;

/// The action waiting for the user to confirm.
pub(super) enum Pending {
    Install {
        source: String,
        files: FileMap,
        name: String,
        update: bool,
    },
    Remove(String),
    Fix(Finding),
    FixAllLinks(Vec<std::path::PathBuf>),
}

#[derive(Default)]
pub(super) struct ActionState {
    pending: Option<Pending>,
    /// Backup of the last completed action, for Undo.
    last_backup: Option<String>,
    busy: bool,
    toast_generation: u64,
    app_update_running: bool,
}

impl Controller {
    // ------------------------------------------------------------ install

    /// Downloads a skill, then asks to confirm installing it.
    pub(super) fn install(&self, source: String, slug: String) {
        let Some(client) = self.directory.clone() else {
            self.show_error("Online features are unavailable.");
            return;
        };
        if self.state.borrow().actions.busy {
            return;
        }
        self.show_toast(
            &format!("Getting {slug} from skills.sh…"),
            true,
            false,
            false,
        );
        let this = self.clone();
        let src = source.clone();
        self.run(
            async move { (client.download(&src, &slug).await, slug) },
            move |_, (result, slug)| match result {
                Ok(Some(files)) => this.confirm_install(source, files, false),
                Ok(None) => this.show_error(&format!(
                    "skills.sh has no downloadable copy of {slug}. Use View to install it another way."
                )),
                Err(e) => this.show_error(&format!("Download failed: {e}")),
            },
        );
    }

    fn confirm_install(&self, source: String, files: FileMap, update: bool) {
        let md = files
            .get("SKILL.md")
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let meta = match skill_md::parse(&md) {
            Ok(m) => m,
            Err(e) => return self.show_error(&format!("This skill's SKILL.md is invalid: {e}")),
        };
        let agents = install_agents(&self.env);
        let names: Vec<&str> = agents.iter().map(|a| a.display_name()).collect();
        let who = if names.is_empty() {
            "No coding agents were detected; it will be saved to ~/.agents/skills.".to_owned()
        } else {
            format!("It will be available to {}.", join_names(&names))
        };
        let scripts = script_files(&files);
        let scripts_note = if scripts.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nIt includes code your agent can run ({}). Review it before trusting a skill from someone you don't know.",
                scripts.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            )
        };
        let name = meta.display_name();
        self.hide_toast();
        if update {
            let state = self
                .state
                .borrow()
                .updates
                .results
                .get(&meta.name)
                .map(|u| (u.state, u.diff.summary()));
            let (state, summary) = state.unwrap_or((UpdateState::UpdateAvailable, String::new()));
            let warning = match state {
                UpdateState::EditedLocally | UpdateState::Diverged | UpdateState::Differs => {
                    "\n\nYour copy has changes that aren't on skills.sh. They'll be replaced, but kept in a backup you can restore."
                }
                _ => "",
            };
            self.ask(
                &format!("Update {name}?"),
                &format!(
                    "{summary}. Your current version is backed up first, so you can undo.{warning}{scripts_note}"
                ),
                "Update",
                false,
                Pending::Install {
                    source,
                    files,
                    name: meta.name,
                    update: true,
                },
            );
        } else {
            self.ask(
                &format!("Install {name}?"),
                &format!(
                    "From {source}, {}. {who}{scripts_note}",
                    plural(files.len(), "file", "files")
                ),
                "Install",
                false,
                Pending::Install {
                    source,
                    files,
                    name: meta.name,
                    update: false,
                },
            );
        }
    }

    /// Updates a skill to the version fetched by the last update check.
    pub(super) fn update_skill(&self, key: &str) {
        let found = {
            let st = self.state.borrow();
            st.updates
                .results
                .get(key)
                .map(|u| (u.source.clone(), u.remote.clone()))
        };
        match found {
            Some((source, files)) => self.confirm_install(source, files, true),
            None => self.check_updates(),
        }
    }

    // ------------------------------------------------------------- remove

    pub(super) fn remove_skill(&self, key: &str) {
        let (name, agents) = {
            let st = self.state.borrow();
            let Some(skill) = st.library.skills.iter().find(|s| s.meta.name == key) else {
                return;
            };
            let mut agents: Vec<&str> = skill
                .installs
                .iter()
                .filter(|i| i.agent != "universal")
                .map(|i| display_name(i.agent))
                .collect();
            agents.sort_unstable();
            agents.dedup();
            (skill.meta.display_name(), join_names(&agents))
        };
        let from = if agents.is_empty() {
            String::new()
        } else {
            format!(" It will no longer be available to {agents}.")
        };
        self.ask(
            &format!("Remove {name}?"),
            &format!("{name} is moved to a backup, not deleted, so you can undo this.{from}"),
            "Remove",
            true,
            Pending::Remove(key.to_owned()),
        );
    }

    // ------------------------------------------------------------ confirm

    pub(super) fn ask(
        &self,
        title: &str,
        message: &str,
        confirm: &str,
        destructive: bool,
        p: Pending,
    ) {
        self.state.borrow_mut().actions.pending = Some(p);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_dialog_title(title.into());
            ui.set_dialog_message(message.into());
            ui.set_dialog_confirm(confirm.into());
            ui.set_dialog_destructive(destructive);
            ui.set_dialog_open(true);
        }
    }

    pub(super) fn dialog_dismissed(&self) {
        self.state.borrow_mut().actions.pending = None;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_dialog_open(false);
        }
    }

    pub(super) fn dialog_accepted(&self) {
        let pending = self.state.borrow_mut().actions.pending.take();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_dialog_open(false);
        }
        let Some(pending) = pending else { return };
        let skill = {
            let st = self.state.borrow();
            match &pending {
                Pending::Remove(key) => st
                    .library
                    .skills
                    .iter()
                    .find(|s| &s.meta.name == key)
                    .cloned(),
                _ => None,
            }
        };
        let (busy, done): (String, &'static str) = match &pending {
            Pending::Install {
                name, update: true, ..
            } => (format!("Updating {name}…"), "Updated"),
            Pending::Install { name, .. } => (format!("Installing {name}…"), "Installed"),
            Pending::Remove(key) => (format!("Removing {key}…"), "Removed"),
            Pending::Fix(_) => ("Fixing…".into(), "Fixed"),
            Pending::FixAllLinks(_) => ("Removing broken links…".into(), "Removed"),
        };
        self.state.borrow_mut().actions.busy = true;
        self.show_toast(&busy, true, false, false);

        let env = Arc::clone(&self.env);
        let agents: Vec<&'static str> = install_agents(&env).iter().map(|a| a.id()).collect();
        let this = self.clone();
        self.run_blocking(
            move || -> Result<Outcome, String> {
                let agents: Vec<Agent> = agents.iter().filter_map(|id| Agent::by_id(id)).collect();
                match pending {
                    Pending::Install { source, files, .. } => {
                        manage::install(&env, &source, &files, &agents)
                    }
                    Pending::Remove(_) => match skill {
                        Some(s) => manage::remove(&env, &s),
                        None => return Err("That skill is no longer installed.".into()),
                    },
                    Pending::Fix(f) => match (f.fix, f.path, f.skill) {
                        (Some(Fix::RemoveBrokenLink), Some(p), _) => {
                            manage::remove_broken_link(&env, &p)
                        }
                        (Some(Fix::RelinkCopy), Some(p), Some(name)) => {
                            let canonical = skills_core::agents::canonical_skills_dir(
                                &env,
                                &skills_core::Scope::Global,
                            )
                            .join(skills_core::sanitize::sanitize_name(&name));
                            manage::relink_copy(&env, &p, &canonical, &name)
                        }
                        _ => return Err("Nothing to fix.".into()),
                    },
                    Pending::FixAllLinks(paths) => manage::remove_broken_links(&env, &paths),
                }
                .map_err(|e| e.to_string())
            },
            move |_, result| {
                this.state.borrow_mut().actions.busy = false;
                match result {
                    Ok(outcome) => {
                        let mut text = format!("{done} {}", outcome.skill);
                        let linked = outcome.linked.len();
                        if done != "Removed" && done != "Fixed" && linked > 0 {
                            text.push_str(&format!(" for {}", plural(linked, "agent", "agents")));
                        }
                        if let Some(note) = &outcome.note {
                            text.push_str(&format!(". {note}"));
                        }
                        this.state.borrow_mut().actions.last_backup = Some(outcome.backup_id);
                        // The installed copy now matches what was published.
                        if done == "Updated" || done == "Installed" || done == "Removed" {
                            this.state
                                .borrow_mut()
                                .updates
                                .results
                                .remove(&outcome.skill);
                        }
                        this.show_toast(&text, false, true, false);
                        this.refresh_library();
                    }
                    Err(e) => this.show_error(&format!("Couldn't finish: {e}")),
                }
            },
        );
    }

    // --------------------------------------------------------------- edit

    /// Opens the SKILL.md editor with the file's current text.
    pub(super) fn edit_start(&self, key: &str) {
        let path = {
            let st = self.state.borrow();
            st.library
                .skills
                .iter()
                .find(|s| s.meta.name == key)
                .map(|s| s.primary_path().join(skill_md::SKILL_FILE))
        };
        let Some(path) = path else { return };
        self.run_blocking(
            move || std::fs::read_to_string(&path).map_err(|e| e.to_string()),
            move |ui, result| match result {
                Ok(text) => {
                    ui.set_edit_text(text.into());
                    ui.set_edit_error("".into());
                    ui.set_edit_open(true);
                }
                Err(e) => ui.set_edit_error(format!("Couldn't open SKILL.md: {e}").into()),
            },
        );
    }

    pub(super) fn edit_save(&self, text: String) {
        let Some(skill) = ({
            let st = self.state.borrow();
            st.detail_key.as_ref().and_then(|k| {
                st.library
                    .skills
                    .iter()
                    .find(|s| &s.meta.name == k)
                    .cloned()
            })
        }) else {
            return;
        };
        // Check before touching disk so mistakes show inline.
        if let Err(e) = manage::validate_skill_md(&text, &skill.meta.name) {
            if let Some(ui) = self.ui.upgrade() {
                ui.set_edit_error(e.into());
            }
            return;
        }
        let env = Arc::clone(&self.env);
        let this = self.clone();
        self.run_blocking(
            move || manage::edit_skill_md(&env, &skill, &text).map_err(|e| e.to_string()),
            move |ui, result| match result {
                Ok(outcome) => {
                    ui.set_edit_open(false);
                    ui.set_edit_error("".into());
                    this.state.borrow_mut().actions.last_backup = Some(outcome.backup_id);
                    this.show_toast(&format!("Saved {}", outcome.skill), false, true, false);
                    this.refresh_library();
                }
                Err(e) => ui.set_edit_error(e.into()),
            },
        );
    }

    pub(super) fn edit_cancel(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_edit_open(false);
            ui.set_edit_error("".into());
        }
    }

    // --------------------------------------------------------------- undo

    pub(super) fn undo_last(&self) {
        let Some(id) = self.state.borrow_mut().actions.last_backup.take() else {
            return;
        };
        self.show_toast("Undoing…", true, false, false);
        let env = Arc::clone(&self.env);
        let this = self.clone();
        self.run_blocking(
            move || manage::undo(&env, &id).map_err(|e| e.to_string()),
            move |_, result| {
                match result {
                    Ok(skill) => {
                        this.state.borrow_mut().updates.results.remove(&skill);
                        this.show_toast(&format!("Restored {skill}"), false, false, false);
                    }
                    Err(e) => this.show_error(&format!("Couldn't undo: {e}")),
                }
                this.refresh_library();
            },
        );
    }

    // -------------------------------------------------------------- toast

    pub(super) fn show_toast(&self, text: &str, busy: bool, undo: bool, error: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_toast_text(text.into());
        ui.set_toast_busy(busy);
        ui.set_toast_undo(undo);
        ui.set_toast_error(error);
        ui.set_toast_open(true);
        let generation = {
            let mut st = self.state.borrow_mut();
            st.actions.toast_generation += 1;
            st.actions.toast_generation
        };
        if !busy {
            let this = self.clone();
            slint::Timer::single_shot(Duration::from_secs(TOAST_SECONDS), move || {
                if this.state.borrow().actions.toast_generation == generation {
                    this.hide_toast();
                }
            });
        }
    }

    pub(super) fn show_error(&self, text: &str) {
        self.show_toast(text, false, false, true);
    }

    pub(super) fn hide_toast(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_toast_open(false);
        }
    }

    // ---------------------------------------------------------- app update

    /// Checks GitHub for a newer SkillsMate. Silent unless `manual` or a newer
    /// version exists.
    pub(super) fn check_app_update(&self, manual: bool) {
        let Some(repo) = release_repo() else {
            if manual {
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_app_update_status("Update checks aren't set up for this build.".into());
                }
            }
            return;
        };
        {
            let mut st = self.state.borrow_mut();
            if st.actions.app_update_running {
                return;
            }
            st.actions.app_update_running = true;
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_app_update_checking(true);
        }
        let state = std::rc::Rc::clone(&self.state);
        self.run(
            async move { latest_release(&repo).await },
            move |ui, result| {
                state.borrow_mut().actions.app_update_running = false;
                ui.set_app_update_checking(false);
                let current = env!("CARGO_PKG_VERSION");
                match result {
                    Ok(Some(r)) if is_newer(&r.version, current) => {
                        ui.set_app_update_status(
                            format!("Version {} is available", r.version).into(),
                        );
                        ui.set_app_update_url(r.url.into());
                    }
                    Ok(_) if manual => {
                        ui.set_app_update_status("You're on the latest version".into())
                    }
                    Err(_) if manual => {
                        ui.set_app_update_status("Couldn't check for updates".into())
                    }
                    _ => {}
                }
            },
        );
    }
}

/// Agents to link a new global skill into: detected agents that support
/// global skills.
fn install_agents(env: &skills_core::Env) -> Vec<Agent> {
    detected_agents(env, None)
        .into_iter()
        .filter(|a| a.id() != "universal" && a.supports_global())
        .collect()
}

fn join_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first @ .., last] if first.len() < 4 => format!("{} and {last}", first.join(", ")),
        _ => format!("{} and {} more", names[..3].join(", "), names.len() - 3),
    }
}
