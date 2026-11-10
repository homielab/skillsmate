//! The CLI's lockfiles.
//!
//! - Global: `$XDG_STATE_HOME/skills/.skill-lock.json`, else
//!   `~/.agents/.skill-lock.json`. Schema version 3.
//! - Project: `<project>/skills-lock.json`. Schema version 1, checked into VCS.
//!
//! Unknown fields are kept (`extra`) and key order is preserved, so a file we
//! write round-trips cleanly through the CLI and produces minimal git diffs.
//! Writes are atomic (temp file + rename in the same directory).
//!
//! Unlike the CLI, we never treat an unreadable or older lockfile as empty:
//! doing so and then writing would wipe it. Callers get a [`LockState`] and
//! decide what to do.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::env::{join_rel, Env};
use crate::error::{Error, Result};

pub const GLOBAL_LOCK_VERSION: u32 = 3;
pub const LOCAL_LOCK_VERSION: u32 = 1;
pub const LOCAL_LOCK_FILE: &str = "skills-lock.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalLock {
    pub version: u32,
    pub skills: IndexMap<String, GlobalLockEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for GlobalLock {
    fn default() -> Self {
        let mut extra = Map::new();
        extra.insert("dismissed".into(), Value::Object(Map::new()));
        Self {
            version: GLOBAL_LOCK_VERSION,
            skills: IndexMap::new(),
            extra,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalLockEntry {
    /// Normalized source, e.g. `owner/repo`.
    pub source: String,
    /// `github`, `local`, `well-known`, ...
    pub source_type: String,
    /// The URL used to install, for re-fetching updates.
    pub source_url: String,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Path of the skill inside the source repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_path: Option<String>,
    /// GitHub tree SHA of the skill folder; changes when any file changes.
    pub skill_folder_hash: String,
    pub installed_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_mode: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalLock {
    pub version: u32,
    pub skills: IndexMap<String, LocalLockEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for LocalLock {
    fn default() -> Self {
        Self {
            version: LOCAL_LOCK_VERSION,
            skills: IndexMap::new(),
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalLockEntry {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_path: Option<String>,
    /// SHA-256 over the skill folder's files on disk.
    pub computed_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_mode: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The result of reading a lockfile.
#[derive(Debug)]
pub enum LockState<T> {
    /// No lockfile yet; safe to create one.
    Missing,
    Loaded(T),
    /// The file is from an older CLI. The CLI discards these; we leave the
    /// decision to the user.
    Outdated {
        version: u32,
    },
    /// Written by a newer CLI than we understand. Never overwrite.
    Newer {
        version: u32,
    },
}

impl<T: Default> LockState<T> {
    /// The lock to modify, if writing back to it is safe.
    pub fn into_writable(self) -> Option<T> {
        match self {
            LockState::Missing => Some(T::default()),
            LockState::Loaded(lock) => Some(lock),
            LockState::Outdated { .. } | LockState::Newer { .. } => None,
        }
    }
}

/// Path of the global lockfile.
pub fn global_lock_path(env: &Env) -> PathBuf {
    match env.path_var("XDG_STATE_HOME") {
        Some(state) => state.join("skills").join(".skill-lock.json"),
        None => join_rel(env.home(), ".agents/.skill-lock.json"),
    }
}

pub fn local_lock_path(project: &Path) -> PathBuf {
    project.join(LOCAL_LOCK_FILE)
}

pub fn read_global(env: &Env) -> Result<LockState<GlobalLock>> {
    read_versioned(&global_lock_path(env), GLOBAL_LOCK_VERSION)
}

pub fn read_local(project: &Path) -> Result<LockState<LocalLock>> {
    read_versioned(&local_lock_path(project), LOCAL_LOCK_VERSION)
}

/// Writes the global lock the way the CLI does: 2-space JSON, no trailing newline.
pub fn write_global(env: &Env, lock: &GlobalLock) -> Result<()> {
    let path = global_lock_path(env);
    let json = serde_json::to_string_pretty(lock).map_err(|e| Error::Json {
        path: path.clone(),
        source: e,
    })?;
    write_atomic(&path, json.as_bytes())
}

/// Writes the project lock the way the CLI does: skills sorted by name,
/// 2-space JSON, trailing newline.
pub fn write_local(project: &Path, lock: &LocalLock) -> Result<()> {
    let path = local_lock_path(project);
    let mut sorted = lock.clone();
    sorted.skills.sort_keys();
    let mut json = serde_json::to_string_pretty(&sorted).map_err(|e| Error::Json {
        path: path.clone(),
        source: e,
    })?;
    json.push('\n');
    write_atomic(&path, json.as_bytes())
}

fn read_versioned<T: DeserializeOwned>(path: &Path, expected: u32) -> Result<LockState<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LockState::Missing),
        Err(e) => return Err(Error::io(path, e)),
    };
    let invalid = |reason: String| Error::InvalidLock {
        path: path.to_path_buf(),
        reason,
    };
    if text.contains("\n<<<<<<<") || text.starts_with("<<<<<<<") {
        return Err(invalid("contains git merge conflict markers".into()));
    }
    let raw: Value = serde_json::from_str(&text).map_err(|e| Error::Json {
        path: path.to_path_buf(),
        source: e,
    })?;
    let version = raw
        .get("version")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| invalid("missing or invalid \"version\"".into()))?;
    if version < expected {
        return Ok(LockState::Outdated { version });
    }
    if version > expected {
        return Ok(LockState::Newer { version });
    }
    if !raw.get("skills").is_some_and(Value::is_object) {
        return Err(invalid("missing \"skills\" map".into()));
    }
    serde_json::from_value(raw)
        .map(LockState::Loaded)
        .map_err(|e| invalid(e.to_string()))
}

/// Replaces `path` atomically so a crash never leaves a half-written file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let dir = path
        .parent()
        .ok_or_else(|| Error::io(path, std::io::Error::other("path has no parent directory")))?;
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| Error::io(dir, e))?;
    tmp.write_all(bytes).map_err(|e| Error::io(tmp.path(), e))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| Error::io(tmp.path(), e))?;
    tmp.persist(path).map_err(|e| Error::io(path, e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOBAL_SAMPLE: &str = r#"{
  "version": 3,
  "skills": {
    "zeta": {
      "source": "vercel-labs/agent-skills",
      "sourceType": "github",
      "sourceUrl": "https://github.com/vercel-labs/agent-skills.git",
      "skillPath": "skills/zeta/SKILL.md",
      "skillFolderHash": "abc123",
      "installedAt": "2026-01-01T00:00:00.000Z",
      "updatedAt": "2026-01-02T00:00:00.000Z",
      "futureField": 1
    },
    "alpha": {
      "source": "o/r",
      "sourceType": "github",
      "sourceUrl": "https://github.com/o/r.git",
      "ref": "v2",
      "skillFolderHash": "def",
      "installedAt": "2026-01-01T00:00:00.000Z",
      "updatedAt": "2026-01-01T00:00:00.000Z"
    }
  },
  "dismissed": {
    "findSkillsPrompt": true
  },
  "lastSelectedAgents": [
    "claude-code"
  ]
}"#;

    fn env_with_home(home: &Path) -> Env {
        Env::new(home.to_path_buf(), [])
    }

    #[test]
    fn global_lock_round_trips_byte_for_byte() {
        let home = tempfile::tempdir().expect("tempdir");
        let env = env_with_home(home.path());
        let path = global_lock_path(&env);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, GLOBAL_SAMPLE).expect("write");

        let LockState::Loaded(lock) = read_global(&env).expect("read") else {
            panic!("expected a loaded lock");
        };
        assert_eq!(
            lock.skills.get_index(0).map(|(k, _)| k.as_str()),
            Some("zeta")
        );
        assert_eq!(lock.skills["alpha"].git_ref.as_deref(), Some("v2"));

        write_global(&env, &lock).expect("write");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            GLOBAL_SAMPLE
        );
    }

    #[test]
    fn missing_and_outdated_and_newer() {
        let home = tempfile::tempdir().expect("tempdir");
        let env = env_with_home(home.path());
        assert!(matches!(
            read_global(&env).expect("read"),
            LockState::Missing
        ));

        let path = global_lock_path(&env);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, r#"{"version":2,"skills":{}}"#).expect("write");
        let state = read_global(&env).expect("read");
        assert!(matches!(state, LockState::Outdated { version: 2 }));
        assert!(state.into_writable().is_none());

        std::fs::write(&path, r#"{"version":9,"skills":{}}"#).expect("write");
        assert!(matches!(
            read_global(&env).expect("read"),
            LockState::Newer { version: 9 }
        ));
    }

    #[test]
    fn conflict_markers_are_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            local_lock_path(dir.path()),
            "{\n<<<<<<< HEAD\n\"version\": 1\n=======\n>>>>>>> b\n}",
        )
        .expect("write");
        assert!(matches!(
            read_local(dir.path()),
            Err(Error::InvalidLock { .. })
        ));
    }

    #[test]
    fn local_lock_is_sorted_with_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lock = LocalLock::default();
        for name in ["b", "a"] {
            lock.skills.insert(
                name.into(),
                LocalLockEntry {
                    source: "o/r".into(),
                    source_url: None,
                    git_ref: None,
                    source_type: "github".into(),
                    skill_path: None,
                    computed_hash: "h".into(),
                    install_mode: None,
                    extra: Map::new(),
                },
            );
        }
        write_local(dir.path(), &lock).expect("write");
        let text = std::fs::read_to_string(local_lock_path(dir.path())).expect("read");
        assert!(text.ends_with("}\n"));
        assert!(text.find("\"a\"") < text.find("\"b\""));
    }

    #[test]
    fn xdg_state_home_moves_global_lock() {
        let state = std::env::temp_dir().join("state");
        let env = Env::new(
            PathBuf::from("/h"),
            [(
                "XDG_STATE_HOME".into(),
                state.to_string_lossy().into_owned(),
            )],
        );
        assert_eq!(
            global_lock_path(&env),
            state.join("skills").join(".skill-lock.json")
        );
    }
}
