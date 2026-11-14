//! Actions that change files: install, update, remove, edit and fixes.
//!
//! Each one is prepared first (`prepare_*` returns the confirmation to show
//! and remembers what was asked), then runs on `confirm_pending`. Every change
//! is backed up, so `undo` can restore it.

use serde::Serialize;
use skills_core::agents::{canonical_skills_dir, detected_agents, Agent};
use skills_core::files::{script_files, FileMap};
use skills_core::health::{pretty_path, Finding, Fix};
use skills_core::manage::{self, UpdateState};
use skills_core::sanitize::sanitize_name;
use skills_core::{skill_md, Env, Scope};

use crate::health::broken_links;
use crate::library::display_name;
use crate::state::{blocking, plural, AppState};

/// The action waiting for the user to confirm.
enum Pending {
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
pub struct ActionState {
    pending: Option<Pending>,
    /// Backup of the last completed action, for Undo.
    last_backup: Option<String>,
}

/// A confirmation dialog.
#[derive(Serialize)]
pub struct Confirm {
    title: String,
    message: String,
    /// Label of the confirm button.
    confirm: String,
    destructive: bool,
    /// What to show while the action runs, e.g. "Installing pdf…".
    busy: String,
}

/// Remembers `pending` and returns the dialog that asks about it.
fn ask(
    state: &AppState,
    title: String,
    message: String,
    confirm: &str,
    destructive: bool,
    pending: Pending,
) -> Confirm {
    let busy = match &pending {
        Pending::Install {
            name, update: true, ..
        } => format!("Updating {name}…"),
        Pending::Install { name, .. } => format!("Installing {name}…"),
        Pending::Remove(key) => format!("Removing {key}…"),
        Pending::Fix(_) => "Fixing…".to_owned(),
        Pending::FixAllLinks(_) => "Removing broken links…".to_owned(),
    };
    state.lock().actions.pending = Some(pending);
    Confirm {
        title,
        message,
        confirm: confirm.to_owned(),
        destructive,
        busy,
    }
}

// ---------------------------------------------------------------- install

/// Downloads a skill from skills.sh, then asks to confirm installing it.
#[tauri::command]
pub async fn prepare_install(
    state: tauri::State<'_, AppState>,
    source: String,
    slug: String,
) -> Result<Confirm, String> {
    let client = state.directory()?;
    let env = state.env()?;
    match client.download(&source, &slug).await {
        Ok(Some(files)) => confirm_install(&state, &env, source, files, false),
        Ok(None) => Err(format!(
            "skills.sh has no downloadable copy of {slug}. Use View to install it another way."
        )),
        Err(e) => Err(format!("Download failed: {e}")),
    }
}

fn confirm_install(
    state: &AppState,
    env: &Env,
    source: String,
    files: FileMap,
    update: bool,
) -> Result<Confirm, String> {
    let md = files
        .get("SKILL.md")
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    let meta =
        skill_md::parse(&md).map_err(|e| format!("This skill's SKILL.md is invalid: {e}"))?;
    let agents = install_agents(env);
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
    if update {
        let checked = state
            .lock()
            .updates
            .results
            .get(&meta.name)
            .map(|u| (u.state, u.diff.summary()));
        let (update_state, summary) =
            checked.unwrap_or((UpdateState::UpdateAvailable, String::new()));
        let warning = match update_state {
            UpdateState::EditedLocally | UpdateState::Diverged | UpdateState::Differs => {
                "\n\nYour copy has changes that aren't on skills.sh. They'll be replaced, but kept in a backup you can restore."
            }
            _ => "",
        };
        let summary = if summary.is_empty() {
            String::new()
        } else {
            format!("{summary}. ")
        };
        Ok(ask(
            state,
            format!("Update {name}?"),
            format!(
                "{summary}Your current version is backed up first, so you can undo.{warning}{scripts_note}"
            ),
            "Update",
            false,
            Pending::Install {
                source,
                files,
                name: meta.name,
                update: true,
            },
        ))
    } else {
        let message = format!(
            "From {source}, {}. {who}{scripts_note}",
            plural(files.len(), "file", "files")
        );
        Ok(ask(
            state,
            format!("Install {name}?"),
            message,
            "Install",
            false,
            Pending::Install {
                source,
                files,
                name: meta.name,
                update: false,
            },
        ))
    }
}

/// Asks to update a skill to the version fetched by the last update check.
/// `None` when that skill hasn't been checked yet: run a check first.
#[tauri::command]
pub async fn prepare_update(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<Option<Confirm>, String> {
    let env = state.env()?;
    let found = state
        .lock()
        .updates
        .results
        .get(&key)
        .map(|u| (u.source.clone(), u.remote.clone()));
    match found {
        Some((source, files)) => confirm_install(&state, &env, source, files, true).map(Some),
        None => Ok(None),
    }
}

// ----------------------------------------------------------------- remove

#[tauri::command]
pub async fn prepare_remove(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<Confirm, String> {
    let (name, agents) = {
        let st = state.lock();
        let skill = st
            .library
            .skills
            .iter()
            .find(|s| s.meta.name == key)
            .ok_or("That skill is no longer installed.")?;
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
    Ok(ask(
        &state,
        format!("Remove {name}?"),
        format!("{name} is moved to a backup, not deleted, so you can undo this.{from}"),
        "Remove",
        true,
        Pending::Remove(key),
    ))
}

// ------------------------------------------------------------------ fixes

/// Asks to run a finding's one-click fix. `index` is the finding's position
/// in the last scan.
#[tauri::command]
pub async fn prepare_fix(
    state: tauri::State<'_, AppState>,
    index: usize,
) -> Result<Confirm, String> {
    let env = state.env()?;
    let finding = state.lock().library.health.findings.get(index).cloned();
    let Some(f) = finding else {
        return Err("That finding is gone. Refresh and try again.".into());
    };
    let (Some(fix), Some(path)) = (f.fix, f.path.clone()) else {
        return Err("Nothing to fix.".into());
    };
    let path = pretty_path(&path, env.home());
    let (title, message, confirm) = match fix {
        Fix::RemoveBrokenLink => (
            "Remove broken link?",
            format!(
                "{path} points to a skill that no longer exists. It will be removed; you can undo this."
            ),
            "Remove link",
        ),
        Fix::RelinkCopy => (
            "Use the main copy?",
            format!(
                "{path} will be replaced with a link to the main copy, so this agent always uses the same version as the others. The old copy is kept in a backup you can restore."
            ),
            "Use main copy",
        ),
    };
    Ok(ask(
        &state,
        title.to_owned(),
        message,
        confirm,
        fix == Fix::RemoveBrokenLink,
        Pending::Fix(f),
    ))
}

/// Asks to remove every broken link the last scan found, in one step.
#[tauri::command]
pub async fn prepare_fix_all_links(state: tauri::State<'_, AppState>) -> Result<Confirm, String> {
    let env = state.env()?;
    let paths = broken_links(&state.lock().library.health.findings);
    if paths.is_empty() {
        return Err("There are no broken links to remove.".into());
    }
    let shown: Vec<String> = paths
        .iter()
        .take(3)
        .map(|p| pretty_path(p, env.home()))
        .collect();
    let more = match paths.len().saturating_sub(shown.len()) {
        0 => String::new(),
        n => format!("\nand {n} more"),
    };
    let n = paths.len();
    Ok(ask(
        &state,
        format!("Remove {n} broken links?"),
        format!(
            "These links point to skills that no longer exist:\n{}{more}\n\nThey'll be removed together; one Undo puts them all back.",
            shown.join("\n")
        ),
        "Remove links",
        true,
        Pending::FixAllLinks(paths),
    ))
}

// ---------------------------------------------------------------- confirm

#[tauri::command]
pub async fn cancel_pending(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.lock().actions.pending = None;
    Ok(())
}

/// Runs the confirmed action. Returns the line to show, e.g. "Installed pdf
/// for 3 agents"; the change can be undone with `undo`.
#[tauri::command]
pub async fn confirm_pending(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let env = state.env()?;
    let (pending, skill) = {
        let mut st = state.lock();
        let Some(pending) = st.actions.pending.take() else {
            return Err("There's nothing to confirm.".into());
        };
        let skill = match &pending {
            Pending::Remove(key) => st
                .library
                .skills
                .iter()
                .find(|s| &s.meta.name == key)
                .cloned(),
            _ => None,
        };
        (pending, skill)
    };
    let done = match &pending {
        Pending::Install { update: true, .. } => "Updated",
        Pending::Install { .. } => "Installed",
        Pending::Remove(_) | Pending::FixAllLinks(_) => "Removed",
        Pending::Fix(_) => "Fixed",
    };

    let outcome = blocking(move || {
        let agents = install_agents(&env);
        match pending {
            Pending::Install { source, files, .. } => {
                manage::install(&env, &source, &files, &agents)
            }
            Pending::Remove(_) => match skill {
                Some(s) => manage::remove(&env, &s),
                None => return Err("That skill is no longer installed.".to_owned()),
            },
            Pending::Fix(f) => match (f.fix, f.path, f.skill) {
                (Some(Fix::RemoveBrokenLink), Some(p), _) => manage::remove_broken_link(&env, &p),
                (Some(Fix::RelinkCopy), Some(p), Some(name)) => {
                    let canonical =
                        canonical_skills_dir(&env, &Scope::Global).join(sanitize_name(&name));
                    manage::relink_copy(&env, &p, &canonical, &name)
                }
                _ => return Err("Nothing to fix.".to_owned()),
            },
            Pending::FixAllLinks(paths) => manage::remove_broken_links(&env, &paths),
        }
        .map_err(|e| e.to_string())
    })
    .await?
    .map_err(|e| format!("Couldn't finish: {e}"))?;

    let mut text = format!("{done} {}", outcome.skill);
    let linked = outcome.linked.len();
    if (done == "Installed" || done == "Updated") && linked > 0 {
        text.push_str(&format!(" for {}", plural(linked, "agent", "agents")));
    }
    if let Some(note) = &outcome.note {
        text.push_str(&format!(". {note}"));
    }
    let mut st = state.lock();
    st.actions.last_backup = Some(outcome.backup_id);
    // The installed copy now matches what was published.
    if done != "Fixed" {
        st.updates.results.remove(&outcome.skill);
    }
    Ok(text)
}

// ------------------------------------------------------------------- edit

/// The current text of a skill's SKILL.md, for the editor.
#[tauri::command]
pub async fn read_skill_md(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<String, String> {
    let path = state
        .lock()
        .library
        .skills
        .iter()
        .find(|s| s.meta.name == key)
        .map(|s| s.primary_path().join(skill_md::SKILL_FILE))
        .ok_or("That skill is no longer installed.")?;
    blocking(move || std::fs::read_to_string(&path))
        .await?
        .map_err(|e| format!("Couldn't open SKILL.md: {e}"))
}

/// Validates and saves an edited SKILL.md. The error is shown beside the
/// editor, so it carries no prefix.
#[tauri::command]
pub async fn save_skill_md(
    state: tauri::State<'_, AppState>,
    key: String,
    text: String,
) -> Result<String, String> {
    let env = state.env()?;
    let skill = state
        .lock()
        .library
        .skills
        .iter()
        .find(|s| s.meta.name == key)
        .cloned()
        .ok_or("That skill is no longer installed.")?;
    // Check before touching disk so mistakes show inline.
    manage::validate_skill_md(&text, &skill.meta.name)?;
    let outcome = blocking(move || manage::edit_skill_md(&env, &skill, &text))
        .await?
        .map_err(|e| e.to_string())?;
    state.lock().actions.last_backup = Some(outcome.backup_id);
    Ok(format!("Saved {}", outcome.skill))
}

// ------------------------------------------------------------------- undo

/// Restores the backup of the last completed action.
#[tauri::command]
pub async fn undo(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let env = state.env()?;
    let id = state
        .lock()
        .actions
        .last_backup
        .take()
        .ok_or("There's nothing to undo.")?;
    let skill = blocking(move || manage::undo(&env, &id))
        .await?
        .map_err(|e| format!("Couldn't undo: {e}"))?;
    state.lock().updates.results.remove(&skill);
    Ok(format!("Restored {skill}"))
}

/// Agents to link a new global skill into: detected agents that support
/// global skills.
fn install_agents(env: &Env) -> Vec<Agent> {
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
