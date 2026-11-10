// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! SkillsMate desktop app. The UI runs on the main thread; all filesystem and
//! network work runs on a tokio runtime and reports back through
//! `upgrade_in_event_loop`, so the window never blocks.

mod controller;
#[cfg(all(target_os = "macos", feature = "app-store"))]
mod home_access;
mod snapshot;

use anyhow::Context;

/// Code generated from `ui/*.slint`. Our lint settings don't apply to it.
mod ui {
    #![allow(clippy::all, clippy::unwrap_used)]
    slint::include_modules!();
}
pub use ui::*;

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("skillsmate-worker")
        .build()
        .context("failed to start the async runtime")?;

    select_backend()?;
    let ui = AppWindow::new().context("failed to create the main window")?;
    // SKILLSMATE_THEME=dark|light overrides the system appearance.
    if let Ok(theme) = std::env::var("SKILLSMATE_THEME") {
        ui.set_theme(theme.trim().to_lowercase().into());
    }
    ui.set_show_app_update(cfg!(not(feature = "app-store")));

    // The controller starts once SkillsMate knows the home folder it may use.
    let controller = std::rc::Rc::new(std::cell::RefCell::new(None));
    let start = {
        let ui = ui.as_weak();
        let rt = runtime.handle().clone();
        let controller = controller.clone();
        move |env: skills_core::Env| -> anyhow::Result<()> {
            let Some(ui) = ui.upgrade() else {
                return Ok(());
            };
            ui.set_needs_access(false);
            let c = controller::Controller::attach(&ui, rt.clone(), env)?;
            *controller.borrow_mut() = Some(c);
            Ok(())
        }
    };
    open_home(&ui, start)?;

    if let Some(dir) = std::env::var_os("SKILLSMATE_SNAPSHOT") {
        snapshot::start(&ui, dir.into());
    }

    ui.run().context("the UI event loop failed")?;
    drop(controller);
    // Dropping the runtime here cancels any in-flight background work.
    drop(runtime);
    Ok(())
}

/// On macOS the content runs under a transparent title bar, like native apps
/// with a sidebar. The UI leaves `Theme.titlebar` free at the top for it.
#[cfg(target_os = "macos")]
fn select_backend() -> anyhow::Result<()> {
    use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(|attributes| {
            attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
        })
        .select()
        .context("failed to start the windowing backend")
}

#[cfg(not(target_os = "macos"))]
fn select_backend() -> anyhow::Result<()> {
    Ok(())
}

/// Starts the app against the user's home folder.
#[cfg(not(all(target_os = "macos", feature = "app-store")))]
fn open_home(
    _ui: &AppWindow,
    start: impl Fn(skills_core::Env) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    start(skills_core::Env::from_system().context("failed to read the environment")?)
}

/// Sandboxed build: restores the home folder access granted earlier, or asks
/// for it. SkillsMate's own data stays in its container.
#[cfg(all(target_os = "macos", feature = "app-store"))]
fn open_home(
    ui: &AppWindow,
    start: impl Fn(skills_core::Env) -> anyhow::Result<()> + 'static,
) -> anyhow::Result<()> {
    fn env_for(home: std::path::PathBuf) -> skills_core::Env {
        let mut vars: Vec<(String, String)> = std::env::vars().collect();
        if let Some(dir) = home_access::container_data_dir() {
            vars.push((
                "SKILLSMATE_DATA_DIR".into(),
                dir.to_string_lossy().into_owned(),
            ));
        }
        skills_core::Env::new(home, vars)
    }

    // Not sandboxed (e.g. `cargo run --features app-store`): nothing to grant.
    if std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_none() {
        return start(skills_core::Env::from_system().context("failed to read the environment")?);
    }
    if let Some(home) = home_access::restore() {
        return start(env_for(home));
    }
    let expected = home_access::real_home().context("couldn't find your home folder")?;
    ui.set_access_home(expected.to_string_lossy().into_owned().into());
    ui.set_needs_access(true);
    let weak = ui.as_weak();
    ui.on_grant_access(move || {
        let Some(ui) = weak.upgrade() else { return };
        match home_access::choose(&expected) {
            Ok(home) => {
                ui.set_access_error("".into());
                if let Err(e) = start(env_for(home)) {
                    ui.set_access_error(format!("Couldn't start: {e}").into());
                }
            }
            Err(message) => ui.set_access_error(message.into()),
        }
    });
    Ok(())
}
