//! Wires UI callbacks to `skills-core`.
//!
//! Background work runs on the tokio runtime and hands plain `Send` data back
//! to the UI thread through [`Controller::run`]; Slint models are built only on
//! the UI thread, since they are not thread-safe. State that the UI thread owns
//! lives in [`State`] behind an `Rc<RefCell<_>>`.

mod actions;
mod audits;
mod detail;
mod explore;
mod health;
mod library;

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use skills_core::directory::DirectoryClient;
use skills_core::search::SearchClient;
use skills_core::Env;
use slint::{ComponentHandle, ModelRc, VecModel};
use tokio::runtime::Handle;

use crate::{AppWindow, Page};

#[derive(Default)]
struct State {
    library: library::LibraryState,
    explore: explore::ExploreState,
    updates: health::UpdatesState,
    actions: actions::ActionState,
    /// Frontmatter name of the skill open in the detail view.
    detail_key: Option<String>,
    audits_running: bool,
}

#[derive(Clone)]
pub struct Controller {
    rt: Handle,
    env: Arc<Env>,
    search: Option<SearchClient>,
    directory: Option<DirectoryClient>,
    search_generation: Arc<AtomicU64>,
    state: Rc<RefCell<State>>,
    ui: slint::Weak<AppWindow>,
}

impl Controller {
    pub fn attach(ui: &AppWindow, rt: Handle, env: Env) -> anyhow::Result<Self> {
        let this = Self {
            rt,
            env: Arc::new(env),
            search: SearchClient::new().ok(),
            directory: DirectoryClient::new().ok(),
            search_generation: Arc::new(AtomicU64::new(0)),
            state: Rc::new(RefCell::new(State::default())),
            ui: ui.as_weak(),
        };

        if this.search.is_none() || this.directory.is_none() {
            ui.set_search_status(
                "Online features are unavailable: could not initialise the network client.".into(),
            );
        }

        let c = this.clone();
        ui.on_refresh(move || c.refresh_library());
        let c = this.clone();
        ui.on_filter_installed(move |text| c.filter_installed(&text));
        let c = this.clone();
        ui.on_check_audits(move || c.check_audits());
        let c = this.clone();
        ui.on_search(move |query| c.search(query.to_string()));
        let c = this.clone();
        ui.on_filter_official(move |text| c.filter_official(&text));
        let c = this.clone();
        ui.on_reload_official(move || c.load_official(true));
        let c = this.clone();
        ui.on_reload_topics(move || c.load_topics(true));
        let c = this.clone();
        ui.on_open_topic(move |slug| c.open_topic(slug.to_string()));
        let c = this.clone();
        ui.on_close_topic(move || c.close_topic());
        let c = this.clone();
        ui.on_page_changed(move |page| c.page_changed(page));
        let c = this.clone();
        ui.on_check_updates(move || c.check_updates());
        let c = this.clone();
        ui.on_update_skill(move |key| c.update_skill(&key));
        let c = this.clone();
        ui.on_fix(move |index| c.fix(index));
        let c = this.clone();
        ui.on_fix_all_links(move || c.fix_all_links());
        let c = this.clone();
        ui.on_open_skill(move |key| c.open_skill(key.to_string()));
        let c = this.clone();
        ui.on_remove_skill(move |key| c.remove_skill(&key));
        let c = this.clone();
        ui.on_install(move |source, slug| c.install(source.to_string(), slug.to_string()));
        let c = this.clone();
        ui.on_dialog_accepted(move || c.dialog_accepted());
        let c = this.clone();
        ui.on_dialog_dismissed(move || c.dialog_dismissed());
        let c = this.clone();
        ui.on_toast_undo_clicked(move || c.undo_last());
        let c = this.clone();
        ui.on_toast_dismissed(move || c.hide_toast());
        let c = this.clone();
        ui.on_check_app_update(move || c.check_app_update(true));
        let c = this.clone();
        ui.on_edit_start(move |key| c.edit_start(&key));
        let c = this.clone();
        ui.on_edit_save(move |text| c.edit_save(text.to_string()));
        let c = this.clone();
        ui.on_edit_cancel(move || c.edit_cancel());
        ui.on_open_url(|url| open_url(&url));

        ui.set_app_version(env!("CARGO_PKG_VERSION").into());
        this.refresh_library();
        this.check_app_update(false);
        Ok(this)
    }

    /// Loads online pages lazily, the first time they are opened.
    fn page_changed(&self, page: Page) {
        match page {
            Page::Official => self.load_official(false),
            Page::Topics => self.load_topics(false),
            Page::Discover => self.load_recommendations(),
            _ => {}
        }
        if page != Page::Detail {
            self.state.borrow_mut().detail_key = None;
            self.edit_cancel();
        }
    }

    /// Runs `work` on the runtime, then `done` with its output on the UI thread
    /// (skipped if the window has closed).
    fn run<T, F, D>(&self, work: F, done: D)
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
        D: FnOnce(&AppWindow, T) + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let _ = tx.send(work.await);
        });
        let weak = self.ui.clone();
        let spawned = slint::spawn_local(async move {
            let Ok(value) = rx.await else { return };
            if let Some(ui) = weak.upgrade() {
                done(&ui, value);
            }
        });
        if let Err(e) = spawned {
            eprintln!("failed to schedule UI update: {e}");
        }
    }

    /// Like [`Controller::run`] for blocking (filesystem) work.
    fn run_blocking<T, F, D>(&self, work: F, done: D)
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
        D: FnOnce(&AppWindow, T) + 'static,
    {
        self.run(
            async move { tokio::task::spawn_blocking(work).await },
            move |ui, r| match r {
                Ok(v) => done(ui, v),
                Err(e) => eprintln!("background task failed: {e}"),
            },
        );
    }
}

pub(crate) fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

pub(crate) fn to_i32(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// "1 skill", "3 skills", "6,802 skills".
pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        let n = skills_core::stats::format_count(n as u64);
        format!("{n} {many}")
    }
}

/// The GitHub repo SkillsMate releases are published to, from Cargo.toml.
pub(crate) fn release_repo() -> Option<String> {
    skills_core::app_update::github_repo(env!("CARGO_PKG_REPOSITORY"))
        .filter(|r| !r.starts_with("your-org/"))
}

/// Opens a link in the default browser. Only skills.sh pages and this app's
/// GitHub releases are allowed, since URLs are built from network data.
fn open_url(url: &str) {
    let releases = release_repo().map(|r| format!("https://github.com/{r}/releases/"));
    let allowed = ["https://www.skills.sh/", "https://skills.sh/"]
        .iter()
        .any(|p| url.starts_with(p))
        || releases.is_some_and(|r| url.starts_with(&r));
    if allowed && !url.contains("..") && !url.chars().any(char::is_whitespace) {
        let _ = webbrowser::open(url);
    }
}
