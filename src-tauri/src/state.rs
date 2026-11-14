//! State shared by all commands.
//!
//! Commands run on Tauri's async runtime. Filesystem work goes through
//! [`blocking`]; the [`State`] lock is only held for short, non-blocking
//! sections and never across an `.await`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use skills_core::directory::DirectoryClient;
use skills_core::search::SearchClient;
use skills_core::Env;

use crate::{actions, explore, health, library};

/// What the app remembers between commands.
#[derive(Default)]
pub struct State {
    pub library: library::LibraryState,
    pub explore: explore::ExploreState,
    pub updates: health::UpdatesState,
    pub actions: actions::ActionState,
}

pub struct AppState {
    /// `None` until SkillsMate knows the home folder it may use (the sandboxed
    /// Mac App Store build asks for it on first launch).
    env: RwLock<Option<Arc<Env>>>,
    pub search: Option<SearchClient>,
    pub directory: Option<DirectoryClient>,
    inner: Mutex<State>,
}

impl AppState {
    pub fn new(env: Option<Env>) -> Self {
        Self {
            env: RwLock::new(env.map(Arc::new)),
            search: SearchClient::new().ok(),
            directory: DirectoryClient::new().ok(),
            inner: Mutex::new(State::default()),
        }
    }

    pub fn env(&self) -> Result<Arc<Env>, String> {
        self.env
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "SkillsMate doesn't have access to your home folder yet.".to_owned())
    }

    pub fn has_env(&self) -> bool {
        self.env
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    #[cfg_attr(not(all(target_os = "macos", feature = "app-store")), allow(dead_code))]
    pub fn set_env(&self, env: Env) {
        *self.env.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(env));
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn directory(&self) -> Result<DirectoryClient, String> {
        self.directory
            .clone()
            .ok_or_else(|| "Online features are unavailable.".to_owned())
    }
}

/// Runs blocking (filesystem) work off the async runtime's worker threads.
pub async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("A background task failed: {e}"))
}

/// "1 skill", "3 skills", "6,802 skills".
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        let n = skills_core::stats::format_count(n as u64);
        format!("{n} {many}")
    }
}
