// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! SkillsMate desktop app. The UI runs on the main thread; all filesystem and
//! network work runs on a tokio runtime and reports back through
//! `upgrade_in_event_loop`, so the window never blocks.

mod controller;
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
    let env = skills_core::Env::from_system().context("failed to read the environment")?;
    let ui = AppWindow::new().context("failed to create the main window")?;
    // SKILLSMATE_THEME=dark|light overrides the system appearance.
    if let Ok(theme) = std::env::var("SKILLSMATE_THEME") {
        ui.set_theme(theme.trim().to_lowercase().into());
    }

    let _controller = controller::Controller::attach(&ui, runtime.handle().clone(), env)?;
    if let Some(dir) = std::env::var_os("SKILLSMATE_SNAPSHOT") {
        snapshot::start(&ui, dir.into());
    }

    ui.run().context("the UI event loop failed")?;
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
