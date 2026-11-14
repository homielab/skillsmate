//! App-level commands: startup, home folder access, the app's own update
//! check, and opening links.

use serde::Serialize;
use skills_core::app_update::{github_repo, is_newer, latest_release};
use tauri_plugin_opener::OpenerExt;

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    version: &'static str,
    /// "dark" or "light" when `SKILLSMATE_THEME` overrides the system setting.
    theme: Option<String>,
    mac: bool,
    /// False in the Mac App Store build, which the App Store updates.
    can_check_app_update: bool,
    /// False when the network client couldn't start.
    online: bool,
    /// The sandboxed build needs the user to grant access to their home
    /// folder before anything else works.
    needs_access: bool,
    /// The home folder to ask for, when `needs_access`.
    access_home: String,
}

/// `SKILLSMATE_THEME=dark|light` overrides the system appearance.
pub fn theme_override() -> Option<String> {
    let theme = std::env::var("SKILLSMATE_THEME")
        .ok()?
        .trim()
        .to_lowercase();
    matches!(theme.as_str(), "dark" | "light").then_some(theme)
}

#[tauri::command]
pub fn bootstrap(state: tauri::State<'_, AppState>) -> Bootstrap {
    let needs_access = !state.has_env();
    Bootstrap {
        version: env!("CARGO_PKG_VERSION"),
        theme: theme_override(),
        mac: cfg!(target_os = "macos"),
        can_check_app_update: release_repo().is_some(),
        online: state.search.is_some() && state.directory.is_some(),
        needs_access,
        access_home: if needs_access {
            crate::access_home()
        } else {
            String::new()
        },
    }
}

/// Opens the folder picker so the user can grant access to their home folder
/// (sandboxed Mac App Store build). Not `async`: the picker must run on the
/// main thread. An empty error means the user cancelled.
#[tauri::command]
pub fn grant_access(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let env = crate::choose_home()?;
    state.set_env(env);
    Ok(())
}

/// The GitHub repo SkillsMate releases are published to, from Cargo.toml.
/// `None` in the Mac App Store build, which the App Store updates.
fn release_repo() -> Option<String> {
    if cfg!(feature = "app-store") {
        return None;
    }
    github_repo(env!("CARGO_PKG_REPOSITORY"))
}

#[derive(Serialize, Default)]
pub struct AppUpdate {
    /// What to tell the user, "" for nothing.
    status: String,
    /// The release page of a newer version, "" when there's none.
    url: String,
}

/// Checks GitHub for a newer SkillsMate. Says nothing unless `manual` or a
/// newer version exists.
#[tauri::command]
pub async fn check_app_update(manual: bool) -> Result<AppUpdate, String> {
    let say = |status: &str| AppUpdate {
        status: if manual {
            status.to_owned()
        } else {
            String::new()
        },
        url: String::new(),
    };
    let Some(repo) = release_repo() else {
        return Ok(say("Update checks aren't set up for this build."));
    };
    Ok(match latest_release(&repo).await {
        Ok(Some(r)) if is_newer(&r.version, env!("CARGO_PKG_VERSION")) => AppUpdate {
            status: format!("Version {} is available", r.version),
            url: r.url,
        },
        Ok(_) => say("You're on the latest version"),
        Err(_) => say("Couldn't check for updates"),
    })
}

/// Opens a link in the default browser. Only skills.sh pages and this app's
/// GitHub releases are allowed, since URLs are built from network data.
#[tauri::command]
pub fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let releases = release_repo().map(|r| format!("https://github.com/{r}/releases/"));
    let allowed = ["https://www.skills.sh/", "https://skills.sh/"]
        .iter()
        .any(|p| url.starts_with(p))
        || releases.is_some_and(|r| url.starts_with(&r));
    if !allowed || url.contains("..") || url.chars().any(char::is_whitespace) {
        return Err("SkillsMate only opens skills.sh and its own release pages.".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("Couldn't open the link: {e}"))
}
