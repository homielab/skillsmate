// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! SkillsMate desktop app: a Tauri shell around `skills-core`.
//!
//! The web frontend (`../src`) only renders. Everything it shows is built by
//! the commands in these modules, which run on Tauri's async runtime so the
//! window never blocks.

mod actions;
mod app;
mod audits;
mod detail;
mod explore;
mod health;
#[cfg(all(target_os = "macos", feature = "app-store"))]
mod home_access;
mod library;
mod state;

use skills_core::Env;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state::AppState::new(initial_env()))
        .setup(|app| {
            if let (Some(theme), Some(window)) =
                (app::theme_override(), app.get_webview_window("main"))
            {
                let theme = match theme.as_str() {
                    "dark" => tauri::Theme::Dark,
                    _ => tauri::Theme::Light,
                };
                let _ = window.set_theme(Some(theme));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::bootstrap,
            app::grant_access,
            app::check_app_update,
            app::open_url,
            library::refresh,
            health::check_updates,
            detail::detail,
            audits::check_audits,
            explore::search,
            explore::official,
            explore::topics,
            explore::topic,
            explore::recommendations,
            actions::prepare_install,
            actions::prepare_update,
            actions::prepare_remove,
            actions::prepare_fix,
            actions::prepare_fix_all_links,
            actions::cancel_pending,
            actions::confirm_pending,
            actions::read_skill_md,
            actions::save_skill_md,
            actions::undo,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start SkillsMate");
}

// ---- Home folder: the user's own, except in the sandboxed App Store build.

/// The environment to start with, or `None` when the user must grant access
/// to their home folder first.
#[cfg(not(all(target_os = "macos", feature = "app-store")))]
fn initial_env() -> Option<Env> {
    match Env::from_system() {
        Ok(env) => Some(env),
        Err(e) => {
            eprintln!("skillsmate: {e}");
            None
        }
    }
}

#[cfg(not(all(target_os = "macos", feature = "app-store")))]
pub(crate) fn access_home() -> String {
    String::new()
}

#[cfg(not(all(target_os = "macos", feature = "app-store")))]
pub(crate) fn choose_home() -> Result<Env, String> {
    Env::from_system().map_err(|e| format!("Couldn't find your home folder: {e}"))
}

/// Sandboxed build: restores the home folder access granted earlier.
/// SkillsMate's own data stays in its container.
#[cfg(all(target_os = "macos", feature = "app-store"))]
fn initial_env() -> Option<Env> {
    // Not sandboxed (e.g. `tauri dev --features app-store`): nothing to grant.
    if std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_none() {
        return Env::from_system().ok();
    }
    home_access::restore().map(sandbox_env)
}

#[cfg(all(target_os = "macos", feature = "app-store"))]
fn sandbox_env(home: std::path::PathBuf) -> Env {
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    if let Some(dir) = home_access::container_data_dir() {
        vars.push((
            "SKILLSMATE_DATA_DIR".into(),
            dir.to_string_lossy().into_owned(),
        ));
    }
    Env::new(home, vars)
}

#[cfg(all(target_os = "macos", feature = "app-store"))]
pub(crate) fn access_home() -> String {
    home_access::real_home()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(all(target_os = "macos", feature = "app-store"))]
pub(crate) fn choose_home() -> Result<Env, String> {
    let expected = home_access::real_home().ok_or("Couldn't find your home folder.")?;
    home_access::choose(&expected).map(sandbox_env)
}
