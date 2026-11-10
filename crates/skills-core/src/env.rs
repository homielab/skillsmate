//! The environment the core resolves paths against.
//!
//! Everything that depends on the user's machine (home directory, XDG
//! variables, agent-specific `*_HOME` overrides) goes through [`Env`], so tests
//! can point the whole crate at a temporary directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Where a skill is installed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The user's home directory (`skills add -g`).
    Global,
    /// A project directory (the CLI's default scope).
    Project(PathBuf),
}

impl Scope {
    pub fn is_global(&self) -> bool {
        matches!(self, Scope::Global)
    }
}

#[derive(Debug, Clone)]
pub struct Env {
    home: PathBuf,
    config_home: PathBuf,
    data_dir: PathBuf,
    vars: HashMap<String, String>,
}

impl Env {
    /// Reads the real environment.
    pub fn from_system() -> Result<Self> {
        let home = dirs::home_dir().ok_or(Error::NoHomeDir)?;
        Ok(Self::new(home, std::env::vars()))
    }

    /// Builds an environment from an explicit home directory and variable set.
    pub fn new(home: PathBuf, vars: impl IntoIterator<Item = (String, String)>) -> Self {
        let vars: HashMap<String, String> = vars.into_iter().collect();
        // The CLI uses `xdg-basedir`, which applies `$XDG_CONFIG_HOME` or
        // `~/.config` on *every* platform, including macOS and Windows. We must
        // match that rather than using the platform config dir, or we'd look
        // for agents in the wrong place.
        let config_home =
            absolute_var(&vars, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let data_dir = absolute_var(&vars, "SKILLSMATE_DATA_DIR")
            .unwrap_or_else(|| platform_data_dir(&home, &vars).join("SkillsMate"));
        Self {
            home,
            config_home,
            data_dir,
            vars,
        }
    }

    /// SkillsMate's own data folder (backups, baselines). Overridable with
    /// `SKILLSMATE_DATA_DIR`.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn config_home(&self) -> &Path {
        &self.config_home
    }

    /// A trimmed, non-empty environment variable.
    pub fn var(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    }

    /// An environment variable that holds an absolute path. Relative values are
    /// ignored: the CLI would resolve them against its working directory, which
    /// has no meaning for a GUI.
    pub fn path_var(&self, key: &str) -> Option<PathBuf> {
        absolute_var(&self.vars, key)
    }

    /// The directory a scope is rooted at.
    pub fn scope_base<'a>(&'a self, scope: &'a Scope) -> &'a Path {
        match scope {
            Scope::Global => &self.home,
            Scope::Project(dir) => dir,
        }
    }
}

/// The per-user data directory, resolved from `home` and the environment the
/// same way the `dirs` crate does, so tests with a fake home stay inside it.
fn platform_data_dir(home: &Path, vars: &HashMap<String, String>) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        absolute_var(vars, "APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming"))
    } else {
        absolute_var(vars, "XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"))
    }
}

fn absolute_var(vars: &HashMap<String, String>, key: &str) -> Option<PathBuf> {
    let value = vars.get(key)?.trim();
    let path = PathBuf::from(value);
    (!value.is_empty() && path.is_absolute()).then_some(path)
}

/// Joins a `/`-separated relative path from the upstream registry onto `base`
/// one component at a time, so the result uses the platform's separator.
pub(crate) fn join_rel(base: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .filter(|c| !c.is_empty())
        .fold(base.to_path_buf(), |acc, c| acc.join(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(p: &str) -> String {
        // An absolute path on any platform.
        std::env::temp_dir().join(p).to_string_lossy().into_owned()
    }

    #[test]
    fn config_home_defaults_to_dot_config() {
        let env = Env::new(PathBuf::from("/h"), []);
        assert_eq!(env.config_home(), Path::new("/h").join(".config"));
    }

    #[test]
    fn config_home_respects_absolute_xdg() {
        let xdg = abs("xdg");
        let env = Env::new(
            PathBuf::from("/h"),
            [("XDG_CONFIG_HOME".into(), xdg.clone())],
        );
        assert_eq!(env.config_home(), Path::new(&xdg));
    }

    #[test]
    fn relative_xdg_is_ignored() {
        let env = Env::new(
            PathBuf::from("/h"),
            [("XDG_CONFIG_HOME".into(), "rel".into())],
        );
        assert_eq!(env.config_home(), Path::new("/h").join(".config"));
    }

    #[test]
    fn join_rel_splits_components() {
        let p = join_rel(Path::new("base"), ".gemini/antigravity/skills");
        assert_eq!(
            p,
            Path::new("base")
                .join(".gemini")
                .join("antigravity")
                .join("skills")
        );
    }
}
