//! Registry of supported coding agents and where each one reads skills from.
//!
//! The data in [`table`] is generated from the official CLI so install paths
//! stay identical. This module only adds the logic for resolving them.

mod table;

use std::path::{Path, PathBuf};

use crate::env::{join_rel, Env, Scope};

/// The directory every skill is stored in, relative to the scope base.
pub const CANONICAL_SKILLS_DIR: &str = ".agents/skills";

/// Stable identifier of an agent, e.g. `claude-code`, as used by `--agent`.
pub type AgentId = &'static str;

/// Where an agent's global skills directory lives.
#[derive(Debug, Clone, Copy)]
pub(crate) enum GlobalDir {
    /// `~/<rel>`
    Home(&'static str),
    /// `$XDG_CONFIG_HOME/<rel>` (or `~/.config/<rel>`)
    Config(&'static str),
    /// `$<var>/<rel>`, falling back to `~/<fallback>/<rel>`.
    EnvHome {
        var: &'static str,
        fallback: &'static str,
        rel: &'static str,
    },
    /// OpenClaw uses the first of `~/.openclaw`, `~/.clawdbot`, `~/.moltbot`
    /// that exists, defaulting to `~/.openclaw`.
    OpenClaw,
}

/// A filesystem marker whose presence means an agent is installed.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Marker {
    Home(&'static str),
    Config(&'static str),
    EnvHome {
        var: &'static str,
        fallback: &'static str,
    },
    /// `$<var>/<rel>`, only checked when the variable is set.
    EnvJoin {
        var: &'static str,
        rel: &'static str,
    },
    /// Relative to the project directory; only checked for project scope.
    Project(&'static str),
    /// A fixed absolute path, such as an app bundle in `/Applications`.
    Absolute(&'static str),
    /// An `agent/` dir plus an `eve` dependency in `package.json`.
    EveProject,
}

#[derive(Debug)]
pub(crate) struct AgentDef {
    pub id: AgentId,
    pub display_name: &'static str,
    pub project_dir: &'static str,
    pub global_dir: Option<GlobalDir>,
    pub detect: &'static [Marker],
    pub hidden_from_universal_list: bool,
    pub create_project_dir_by_default: bool,
}

/// A supported agent.
#[derive(Debug, Clone, Copy)]
pub struct Agent(&'static AgentDef);

impl Agent {
    /// All agents, in the CLI's order.
    pub fn all() -> impl Iterator<Item = Agent> {
        table::AGENTS.iter().map(Agent)
    }

    pub fn by_id(id: &str) -> Option<Agent> {
        Self::all().find(|a| a.id() == id)
    }

    pub fn id(self) -> AgentId {
        self.0.id
    }

    pub fn display_name(self) -> &'static str {
        self.0.display_name
    }

    /// Universal agents read the canonical `.agents/skills` directory directly,
    /// so installing for them needs no symlink.
    pub fn is_universal(self) -> bool {
        self.0.project_dir == CANONICAL_SKILLS_DIR
    }

    /// Whether this agent is listed among universal agents in the UI. The
    /// `universal` pseudo-agent itself is hidden.
    pub fn shown_in_universal_list(self) -> bool {
        self.is_universal() && !self.0.hidden_from_universal_list
    }

    pub fn supports_global(self) -> bool {
        self.0.global_dir.is_some()
    }

    pub fn creates_project_dir_by_default(self) -> bool {
        self.0.create_project_dir_by_default
    }

    /// The directory this agent loads skills from in `scope`, or `None` if
    /// the agent has no global skills directory.
    ///
    /// Matches the CLI's `getAgentBaseDir`: universal agents always resolve to
    /// the canonical directory.
    pub fn skills_dir(self, env: &Env, scope: &Scope) -> Option<PathBuf> {
        if self.is_universal() {
            return Some(canonical_skills_dir(env, scope));
        }
        match scope {
            Scope::Project(dir) => Some(join_rel(dir, self.0.project_dir)),
            Scope::Global => self.0.global_dir.map(|g| resolve_global(env, g)),
        }
    }

    /// The agent's own global skills directory, when it differs from the one
    /// [`Agent::skills_dir`] installs to. Universal agents such as Cursor
    /// install into `~/.agents/skills` but also load `~/.cursor/skills`, where
    /// older CLI versions or the user may have put skills, so scans include it.
    pub fn native_global_dir(self, env: &Env) -> Option<PathBuf> {
        let native = self.0.global_dir.map(|g| resolve_global(env, g))?;
        (native != canonical_skills_dir(env, &Scope::Global)).then_some(native)
    }

    /// Whether the agent appears to be installed on this machine. Project
    /// markers are only checked when `project` is given.
    pub fn is_detected(self, env: &Env, project: Option<&Path>) -> bool {
        self.0
            .detect
            .iter()
            .any(|m| marker_exists(env, project, *m))
    }
}

/// `<base>/.agents/skills` for the given scope.
pub fn canonical_skills_dir(env: &Env, scope: &Scope) -> PathBuf {
    join_rel(env.scope_base(scope), CANONICAL_SKILLS_DIR)
}

/// Agents that appear to be installed.
pub fn detected_agents(env: &Env, project: Option<&Path>) -> Vec<Agent> {
    Agent::all()
        .filter(|a| a.is_detected(env, project))
        .collect()
}

fn env_home(env: &Env, var: &str, fallback: &str) -> PathBuf {
    env.path_var(var)
        .unwrap_or_else(|| join_rel(env.home(), fallback))
}

fn resolve_global(env: &Env, dir: GlobalDir) -> PathBuf {
    match dir {
        GlobalDir::Home(rel) => join_rel(env.home(), rel),
        GlobalDir::Config(rel) => join_rel(env.config_home(), rel),
        GlobalDir::EnvHome { var, fallback, rel } => join_rel(&env_home(env, var, fallback), rel),
        GlobalDir::OpenClaw => {
            let root = [".openclaw", ".clawdbot", ".moltbot"]
                .iter()
                .map(|d| env.home().join(d))
                .find(|p| p.exists())
                .unwrap_or_else(|| env.home().join(".openclaw"));
            root.join("skills")
        }
    }
}

fn marker_exists(env: &Env, project: Option<&Path>, marker: Marker) -> bool {
    match marker {
        Marker::Home(rel) => join_rel(env.home(), rel).exists(),
        Marker::Config(rel) => join_rel(env.config_home(), rel).exists(),
        Marker::EnvHome { var, fallback } => env_home(env, var, fallback).exists(),
        Marker::EnvJoin { var, rel } => env
            .path_var(var)
            .is_some_and(|p| join_rel(&p, rel).exists()),
        Marker::Project(rel) => project.is_some_and(|p| join_rel(p, rel).exists()),
        Marker::Absolute(path) => Path::new(path).exists(),
        Marker::EveProject => project.is_some_and(is_eve_project),
    }
}

fn is_eve_project(project: &Path) -> bool {
    if !project.join("agent").is_dir() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(project.join("package.json")) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    ["dependencies", "devDependencies"].iter().any(|k| {
        json.get(k)
            .and_then(|d| d.get("eve"))
            .is_some_and(|v| !v.is_null())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(home: &Path) -> Env {
        Env::new(home.to_path_buf(), [])
    }

    #[test]
    fn registry_matches_upstream_size_and_ids_are_unique() {
        let ids: Vec<_> = Agent::all().map(Agent::id).collect();
        assert_eq!(ids.len(), 79);
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn universal_agents_use_canonical_dir() {
        let home = Path::new("/home/u");
        let codex = Agent::by_id("codex").expect("codex exists");
        assert!(codex.is_universal());
        assert_eq!(
            codex.skills_dir(&env(home), &Scope::Global),
            Some(home.join(".agents").join("skills"))
        );
    }

    #[test]
    fn claude_code_respects_config_dir_override() {
        let home = Path::new("/home/u");
        let custom = std::env::temp_dir().join("claude-custom");
        let e = Env::new(
            home.to_path_buf(),
            [(
                "CLAUDE_CONFIG_DIR".into(),
                custom.to_string_lossy().into_owned(),
            )],
        );
        let claude = Agent::by_id("claude-code").expect("claude-code exists");
        assert_eq!(
            claude.skills_dir(&e, &Scope::Global),
            Some(custom.join("skills"))
        );
        assert_eq!(
            claude.skills_dir(&env(home), &Scope::Global),
            Some(home.join(".claude").join("skills"))
        );
    }

    #[test]
    fn project_scope_joins_project_dir() {
        let project = Path::new("/work/app");
        let windsurf = Agent::by_id("windsurf").expect("windsurf exists");
        assert_eq!(
            windsurf.skills_dir(&env(Path::new("/h")), &Scope::Project(project.into())),
            Some(project.join(".windsurf").join("skills"))
        );
    }

    #[test]
    fn cursor_is_universal_but_keeps_native_global_dir() {
        let home = Path::new("/home/u");
        let cursor = Agent::by_id("cursor").expect("cursor exists");
        assert!(cursor.is_universal());
        assert_eq!(
            cursor.skills_dir(&env(home), &Scope::Global),
            Some(home.join(".agents").join("skills"))
        );
        assert_eq!(
            cursor.native_global_dir(&env(home)),
            Some(home.join(".cursor").join("skills"))
        );
    }

    #[test]
    fn agents_without_global_dir_return_none() {
        let eve = Agent::by_id("eve").expect("eve exists");
        assert!(!eve.supports_global());
        assert_eq!(eve.skills_dir(&env(Path::new("/h")), &Scope::Global), None);
    }

    #[test]
    fn detection_uses_home_markers() {
        let home = tempfile::tempdir().expect("tempdir");
        let cursor = Agent::by_id("cursor").expect("cursor exists");
        assert!(!cursor.is_detected(&env(home.path()), None));
        std::fs::create_dir(home.path().join(".cursor")).expect("mkdir");
        assert!(cursor.is_detected(&env(home.path()), None));
    }
}
