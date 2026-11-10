//! Changing installed skills: install, update, remove, one-click fixes, and
//! undo.
//!
//! Safety rules:
//! - Nothing is deleted outright. Anything replaced or removed is first moved
//!   into a backup under `<data dir>/backups/<id>/`, described by a
//!   `manifest.json` that [`undo`] uses to put everything back.
//! - Every path touched must sit directly inside a known skills folder (the
//!   canonical `~/.agents/skills` or an agent's global skills folder).
//! - New files are written to a temporary folder next to the target and moved
//!   into place in one rename, so agents never see a half-written skill.
//! - On failure mid-way, completed steps are rolled back from the backup.
//! - The global lockfile is only written when it is safely writable (see
//!   [`LockState::into_writable`]); it keeps the CLI's format.
//!
//! Global scope only for now. Blocking; run off the UI thread.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Map;

use crate::agents::{canonical_skills_dir, Agent, AgentId};
use crate::env::{join_rel, Env, Scope};
use crate::error::{Error, Result};
use crate::files::{content_hash, validate_snapshot_path, FileMap};
use crate::lock::{read_global, write_atomic, write_global, GlobalLock, GlobalLockEntry};
use crate::sanitize::sanitize_name;
use crate::scan::{InstalledSkill, LinkKind};
use crate::skill_md;
use crate::stats::now_iso;

/// Backups kept; older ones are pruned after each operation.
const KEEP_BACKUPS: usize = 30;

// ----------------------------------------------------------- update state ---

/// How an installed skill compares with its published version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateState {
    UpToDate,
    /// The published version changed; your copy is untouched since install.
    UpdateAvailable,
    /// You changed your copy; the published version is unchanged.
    EditedLocally,
    /// Both changed since SkillsMate installed it.
    Diverged,
    /// Different, and there's no record of what was installed (installed by
    /// the CLI or by hand), so we can't tell which side changed.
    Differs,
}

impl UpdateState {
    pub fn label(self) -> &'static str {
        match self {
            Self::UpToDate => "Up to date",
            Self::UpdateAvailable => "Update available",
            Self::EditedLocally => "Edited locally",
            Self::Diverged => "Edited locally and updated",
            Self::Differs => "Differs from skills.sh",
        }
    }
}

pub fn compare(local: &FileMap, remote: &FileMap, baseline: Option<&str>) -> UpdateState {
    if local == remote {
        return UpdateState::UpToDate;
    }
    let Some(base) = baseline else {
        return UpdateState::Differs;
    };
    let (l, r) = (content_hash(local), content_hash(remote));
    match (l == base, r == base) {
        (true, false) => UpdateState::UpdateAvailable,
        (false, true) => UpdateState::EditedLocally,
        _ => UpdateState::Diverged,
    }
}

// -------------------------------------------------------------- baselines ---

/// Hash of what SkillsMate last installed for each skill, so later checks can
/// tell your edits apart from upstream updates.
pub fn load_baselines(env: &Env) -> BTreeMap<String, String> {
    std::fs::read_to_string(env.data_dir().join("baselines.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn set_baseline(env: &Env, name: &str, hash: Option<String>) -> Result<()> {
    let mut all = load_baselines(env);
    match hash {
        Some(h) => all.insert(name.to_owned(), h),
        None => all.remove(name),
    };
    let json = serde_json::to_string_pretty(&all).map_err(|e| Error::Json {
        path: env.data_dir().join("baselines.json"),
        source: e,
    })?;
    write_atomic(&env.data_dir().join("baselines.json"), json.as_bytes())
}

// ---------------------------------------------------------------- backups ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    original: PathBuf,
    /// `dir` (moved into the backup) or `symlink` (target recorded).
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stored: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    id: String,
    action: String,
    skill: String,
    created_at: String,
    items: Vec<Item>,
    /// Paths this operation created; undo removes them.
    created: Vec<PathBuf>,
    /// The lock entry before the operation (`None` = there was none).
    lock_before: Option<serde_json::Value>,
    lock_touched: bool,
    baseline_before: Option<String>,
}

/// What an operation did, for the UI's confirmation and Undo.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub backup_id: String,
    pub skill: String,
    /// Agents that got a link to the skill.
    pub linked: Vec<AgentId>,
    /// Set when the lockfile couldn't be updated safely.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackupInfo {
    pub id: String,
    pub action: String,
    pub skill: String,
    pub created_at: String,
}

struct Backup<'a> {
    env: &'a Env,
    dir: PathBuf,
    m: Manifest,
}

impl<'a> Backup<'a> {
    fn new(env: &'a Env, action: &str, skill: &str) -> Result<Self> {
        let root = backups_dir(env);
        std::fs::create_dir_all(&root).map_err(|e| Error::io(&root, e))?;
        let stamp: String = now_iso()
            .chars()
            .filter(char::is_ascii_digit)
            .take(14)
            .collect();
        let base = format!("{stamp}-{action}-{}", sanitize_name(skill));
        let mut id = base.clone();
        let mut n = 1;
        while root.join(&id).exists() {
            n += 1;
            id = format!("{base}-{n}");
        }
        let dir = root.join(&id);
        std::fs::create_dir(&dir).map_err(|e| Error::io(&dir, e))?;
        Ok(Self {
            env,
            dir,
            m: Manifest {
                version: 1,
                id,
                action: action.into(),
                skill: skill.into(),
                created_at: now_iso(),
                items: Vec::new(),
                created: Vec::new(),
                lock_before: None,
                lock_touched: false,
                baseline_before: None,
            },
        })
    }

    /// Moves a skill folder or link out of the way, into the backup.
    fn stash(&mut self, path: &Path) -> Result<()> {
        ensure_managed(self.env, path)?;
        let meta = path.symlink_metadata().map_err(|e| Error::io(path, e))?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path).map_err(|e| Error::io(path, e))?;
            std::fs::remove_file(path).map_err(|e| Error::io(path, e))?;
            self.m.items.push(Item {
                original: path.to_path_buf(),
                kind: "symlink".into(),
                target: Some(target),
                stored: None,
            });
        } else if meta.is_dir() {
            let stored = format!("item-{}", self.m.items.len());
            move_dir(path, &self.dir.join(&stored))?;
            self.m.items.push(Item {
                original: path.to_path_buf(),
                kind: "dir".into(),
                target: None,
                stored: Some(stored),
            });
        } else {
            return Err(Error::Parse(format!(
                "{} is not a skill folder or link",
                path.display()
            )));
        }
        self.save()
    }

    fn created(&mut self, path: &Path) -> Result<()> {
        self.m.created.push(path.to_path_buf());
        self.save()
    }

    fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.m).map_err(|e| Error::Json {
            path: self.dir.join("manifest.json"),
            source: e,
        })?;
        write_atomic(&self.dir.join("manifest.json"), json.as_bytes())
    }

    fn outcome(self, linked: Vec<AgentId>, note: Option<String>) -> Outcome {
        prune_backups(self.env);
        Outcome {
            backup_id: self.m.id,
            skill: self.m.skill,
            linked,
            note,
        }
    }

    /// Best-effort rollback after a failure part-way through.
    fn rollback(self) {
        let id = self.m.id.clone();
        let _ = self.save();
        let _ = undo(self.env, &id);
    }
}

fn backups_dir(env: &Env) -> PathBuf {
    env.data_dir().join("backups")
}

/// Backups, newest first.
pub fn list_backups(env: &Env) -> Vec<BackupInfo> {
    let mut out: Vec<BackupInfo> = std::fs::read_dir(backups_dir(env))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read_manifest(&e.path()).ok())
        .map(|m| BackupInfo {
            id: m.id,
            action: m.action,
            skill: m.skill,
            created_at: m.created_at,
        })
        .collect();
    out.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    out
}

fn read_manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join("manifest.json");
    let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
    serde_json::from_str(&text).map_err(|e| Error::Json { path, source: e })
}

fn prune_backups(env: &Env) {
    for old in list_backups(env).into_iter().skip(KEEP_BACKUPS) {
        let dir = backups_dir(env).join(&old.id);
        if dir.starts_with(backups_dir(env)) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

// ------------------------------------------------------------- operations ---

/// Installs (or updates) a skill from its published files and links it into
/// each of `agents` that needs a link. Replaced folders and links go to a
/// backup first.
pub fn install(env: &Env, source: &str, files: &FileMap, agents: &[Agent]) -> Result<Outcome> {
    let skill_md = files
        .get("SKILL.md")
        .ok_or_else(|| Error::Parse("no SKILL.md".into()))?;
    let meta = skill_md::parse(&String::from_utf8_lossy(skill_md))
        .map_err(|e| Error::Parse(format!("SKILL.md: {e}")))?;
    let dir_name = sanitize_name(&meta.name);
    let base = canonical_skills_dir(env, &Scope::Global);
    std::fs::create_dir_all(&base).map_err(|e| Error::io(&base, e))?;
    let target = base.join(&dir_name);

    let action = if target.exists() || target.symlink_metadata().is_ok() {
        "update"
    } else {
        "install"
    };
    let mut backup = Backup::new(env, action, &meta.name)?;
    match install_steps(env, &mut backup, source, files, &meta.name, &target, agents) {
        Ok((linked, note)) => Ok(backup.outcome(linked, note)),
        Err(e) => {
            backup.rollback();
            Err(e)
        }
    }
}

fn install_steps(
    env: &Env,
    backup: &mut Backup<'_>,
    source: &str,
    files: &FileMap,
    name: &str,
    target: &Path,
    agents: &[Agent],
) -> Result<(Vec<AgentId>, Option<String>)> {
    let base = target
        .parent()
        .ok_or_else(|| Error::Parse("invalid target".into()))?;
    // 1. Write the new version next to the target.
    let tmp = tempfile::Builder::new()
        .prefix(".skillsmate-")
        .tempdir_in(base)
        .map_err(|e| Error::io(base, e))?;
    for (rel, bytes) in files {
        validate_snapshot_path(rel)?;
        let path = join_rel(tmp.path(), rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(&path, bytes).map_err(|e| Error::io(&path, e))?;
    }
    // 2. Move the old version (if any) into the backup, then swap in the new.
    if target.symlink_metadata().is_ok() {
        backup.stash(target)?;
    }
    let tmp_path = tmp.keep();
    std::fs::rename(&tmp_path, target).map_err(|e| Error::io(target, e))?;
    backup.created(target)?;

    // 3. Link into agents that don't read the canonical folder.
    let dir_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut linked = Vec::new();
    for agent in agents {
        if agent.is_universal() {
            linked.push(agent.id());
            continue;
        }
        let Some(agent_dir) = agent.skills_dir(env, &Scope::Global) else {
            continue;
        };
        let link = agent_dir.join(&dir_name);
        if is_link_to(&link, target) {
            linked.push(agent.id());
            continue;
        }
        if link.symlink_metadata().is_ok() {
            backup.stash(&link)?;
        }
        std::fs::create_dir_all(&agent_dir).map_err(|e| Error::io(&agent_dir, e))?;
        link_dir(target, &link)?;
        backup.created(&link)?;
        linked.push(agent.id());
    }

    // 4. Record it like the CLI does, and remember what we installed.
    let note = update_lock(env, backup, name, |previous| {
        let now = now_iso();
        Some(GlobalLockEntry {
            source: source.to_owned(),
            source_type: "github".into(),
            source_url: format!("https://github.com/{source}.git"),
            git_ref: None,
            skill_path: previous.and_then(|p| p.skill_path.clone()),
            // The CLI's update check needs a GitHub tree SHA we don't have;
            // SkillsMate tracks updates with its own baseline instead.
            skill_folder_hash: String::new(),
            installed_at: previous.map_or_else(|| now.clone(), |p| p.installed_at.clone()),
            updated_at: now,
            plugin_name: None,
            install_mode: None,
            extra: Map::new(),
        })
    })?;
    backup.m.baseline_before = load_baselines(env).get(name).cloned();
    backup.save()?;
    set_baseline(env, name, Some(content_hash(files)))?;
    Ok((linked, note))
}

/// Removes a global skill: its main folder, agent links and copies, and its
/// lock entry. Everything goes to a backup.
pub fn remove(env: &Env, skill: &InstalledSkill) -> Result<Outcome> {
    if skill.scope != Scope::Global {
        return Err(Error::Parse(
            "only global skills can be removed for now".into(),
        ));
    }
    let mut backup = Backup::new(env, "remove", &skill.meta.name)?;
    let result = (|| -> Result<Option<String>> {
        for install in &skill.installs {
            if install.kind != LinkKind::Canonical
                && Some(&install.path) != skill.canonical_path.as_ref()
                && install.path.symlink_metadata().is_ok()
            {
                backup.stash(&install.path)?;
            }
        }
        if let Some(c) = &skill.canonical_path {
            if c.symlink_metadata().is_ok() {
                backup.stash(c)?;
            }
        }
        let note = update_lock(env, &mut backup, &skill.meta.name, |_| None)?;
        backup.m.baseline_before = load_baselines(env).get(&skill.meta.name).cloned();
        backup.save()?;
        set_baseline(env, &skill.meta.name, None)?;
        Ok(note)
    })();
    match result {
        Ok(note) => Ok(backup.outcome(Vec::new(), note)),
        Err(e) => {
            backup.rollback();
            Err(e)
        }
    }
}

/// Removes a symlink whose target no longer exists.
pub fn remove_broken_link(env: &Env, path: &Path) -> Result<Outcome> {
    if path.metadata().is_ok() {
        return Err(Error::Parse(format!("{} is not broken", path.display())));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut backup = Backup::new(env, "fix", &name)?;
    match backup.stash(path) {
        Ok(()) => Ok(backup.outcome(Vec::new(), None)),
        Err(e) => {
            backup.rollback();
            Err(e)
        }
    }
}

/// Removes several broken symlinks as one operation: one backup, so a single
/// Undo restores them all. Links that work again by now are left alone.
/// `Outcome::skill` reads like "3 broken links".
pub fn remove_broken_links(env: &Env, paths: &[PathBuf]) -> Result<Outcome> {
    let broken: Vec<&PathBuf> = paths
        .iter()
        .filter(|p| p.symlink_metadata().is_ok() && p.metadata().is_err())
        .collect();
    if broken.is_empty() {
        return Err(Error::Parse("no broken links left to remove".into()));
    }
    let label = match broken.len() {
        1 => "1 broken link".to_owned(),
        n => format!("{n} broken links"),
    };
    let mut backup = Backup::new(env, "fix", &label)?;
    let result = broken.iter().try_for_each(|p| backup.stash(p));
    match result {
        Ok(()) => Ok(backup.outcome(Vec::new(), None)),
        Err(e) => {
            backup.rollback();
            Err(e)
        }
    }
}

/// Replaces an agent's separate copy of a skill with a link to the main copy.
pub fn relink_copy(env: &Env, copy: &Path, canonical: &Path, skill: &str) -> Result<Outcome> {
    ensure_managed(env, canonical)?;
    let mut backup = Backup::new(env, "fix", skill)?;
    let result = (|| -> Result<()> {
        backup.stash(copy)?;
        link_dir(canonical, copy)?;
        backup.created(copy)
    })();
    match result {
        Ok(()) => Ok(backup.outcome(Vec::new(), None)),
        Err(e) => {
            backup.rollback();
            Err(e)
        }
    }
}

/// Replaces a skill's SKILL.md with `text` after validating it. The whole
/// skill folder is copied into a backup first, so Undo restores it exactly.
/// Renaming is refused: the folder, agent links and lock entry are keyed by
/// the name.
pub fn edit_skill_md(env: &Env, skill: &InstalledSkill, text: &str) -> Result<Outcome> {
    let dir = skill
        .canonical_path
        .clone()
        .unwrap_or_else(|| skill.primary_path().to_path_buf());
    validate_skill_md(text, &skill.meta.name).map_err(Error::Parse)?;
    // Edit the real folder, not a link to it.
    let dir = std::fs::canonicalize(&dir).map_err(|e| Error::io(&dir, e))?;
    let real_file = dir.join(skill_md::SKILL_FILE);
    let link_path = skill
        .canonical_path
        .clone()
        .unwrap_or_else(|| skill.primary_path().to_path_buf());
    ensure_managed(env, &link_path)?;

    let mut backup = Backup::new(env, "edit", &skill.meta.name)?;
    let result = (|| -> Result<()> {
        let stored = "item-0".to_owned();
        copy_dir(&dir, &backup.dir.join(&stored))?;
        backup.m.items.push(Item {
            original: link_path.clone(),
            kind: "dir".into(),
            target: None,
            stored: Some(stored),
        });
        // Undo removes the edited folder and moves the copy back. If the
        // skill folder is itself a link, undo restores the folder's contents
        // in place instead (see `undo`).
        backup.m.created.push(link_path.clone());
        backup.save()?;
        write_atomic(&real_file, text.as_bytes())
    })();
    match result {
        Ok(()) => Ok(backup.outcome(Vec::new(), None)),
        Err(e) => {
            backup.rollback();
            Err(e)
        }
    }
}

/// Checks edited SKILL.md text: under 1 MiB, valid frontmatter with `name`
/// and `description`, and the same name as before.
pub fn validate_skill_md(text: &str, expected_name: &str) -> std::result::Result<(), String> {
    if text.len() as u64 > skill_md::MAX_SKILL_MD_BYTES {
        return Err("SKILL.md can't be larger than 1 MiB".into());
    }
    let meta = skill_md::parse(text)?;
    if meta.name != expected_name {
        return Err(format!(
            "The name must stay \"{expected_name}\": its folder, agent links and lockfile entry use it."
        ));
    }
    Ok(())
}

/// Reverses an operation from its backup. Returns the skill name.
pub fn undo(env: &Env, backup_id: &str) -> Result<String> {
    if backup_id.contains(['/', '\\']) || backup_id.contains("..") {
        return Err(Error::Parse("invalid backup id".into()));
    }
    let dir = backups_dir(env).join(backup_id);
    let m = read_manifest(&dir)?;

    // Edits of a skill whose folder is a link: restore the files in place,
    // leaving the link alone.
    if m.action == "edit" {
        if let [item] = m.items.as_slice() {
            if item
                .original
                .symlink_metadata()
                .is_ok_and(|md| md.file_type().is_symlink())
            {
                ensure_managed(env, &item.original)?;
                let real = std::fs::canonicalize(&item.original)
                    .map_err(|e| Error::io(&item.original, e))?;
                let stored = dir.join(item.stored.as_deref().unwrap_or("item-0"));
                let restored = real.with_extension("skillsmate-restore");
                copy_dir(&stored, &restored)?;
                std::fs::remove_dir_all(&real).map_err(|e| Error::io(&real, e))?;
                std::fs::rename(&restored, &real).map_err(|e| Error::io(&real, e))?;
                let _ = std::fs::remove_dir_all(&dir);
                return Ok(m.skill);
            }
        }
    }

    // Remove what the operation created, newest first.
    for path in m.created.iter().rev() {
        ensure_managed(env, path)?;
        match path.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() || meta.is_file() => {
                std::fs::remove_file(path).map_err(|e| Error::io(path, e))?;
            }
            Ok(meta) if meta.is_dir() => {
                std::fs::remove_dir_all(path).map_err(|e| Error::io(path, e))?;
            }
            _ => {}
        }
    }
    // Put back what it moved away, oldest last so nested order is preserved.
    for item in m.items.iter().rev() {
        ensure_managed(env, &item.original)?;
        if item.original.symlink_metadata().is_ok() {
            return Err(Error::Parse(format!(
                "can't restore {}: something is already there",
                item.original.display()
            )));
        }
        if let Some(parent) = item.original.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        match (item.kind.as_str(), &item.target, &item.stored) {
            ("symlink", Some(target), _) => link_raw(target, &item.original)?,
            ("dir", _, Some(stored)) => move_dir(&dir.join(stored), &item.original)?,
            _ => return Err(Error::Parse("corrupt backup manifest".into())),
        }
    }
    if m.lock_touched {
        let before: Option<GlobalLockEntry> = m
            .lock_before
            .clone()
            .and_then(|v| serde_json::from_value(v).ok());
        let mut b = Backup {
            env,
            dir: dir.clone(),
            m: m.clone(),
        };
        update_lock(env, &mut b, &m.skill, |_| before.clone())?;
    }
    set_baseline(env, &m.skill, m.baseline_before.clone())?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(m.skill)
}

// ---------------------------------------------------------------- helpers ---

/// Applies `change` to the skill's global lock entry when the lock is safely
/// writable, recording the previous entry in the backup. Returns a note for
/// the user when it couldn't write.
fn update_lock(
    env: &Env,
    backup: &mut Backup<'_>,
    name: &str,
    change: impl FnOnce(Option<&GlobalLockEntry>) -> Option<GlobalLockEntry>,
) -> Result<Option<String>> {
    let Some(mut lock) = read_global(env)?.into_writable() else {
        return Ok(Some(
            "The skills lockfile is from a different CLI version, so it wasn't changed.".into(),
        ));
    };
    let previous = lock.skills.get(name).cloned();
    if !backup.m.lock_touched {
        backup.m.lock_before = previous.as_ref().and_then(|p| serde_json::to_value(p).ok());
        backup.m.lock_touched = true;
        backup.save()?;
    }
    match change(previous.as_ref()) {
        Some(entry) => {
            lock.skills.insert(name.to_owned(), entry);
        }
        None => {
            lock.skills.shift_remove(name);
        }
    }
    if lock == GlobalLock::default() && previous.is_none() {
        return Ok(None);
    }
    write_global(env, &lock)?;
    Ok(None)
}

/// Every folder SkillsMate may change: the canonical skills folder and each
/// agent's global skills folder.
fn managed_roots(env: &Env) -> Vec<PathBuf> {
    let mut roots = vec![canonical_skills_dir(env, &Scope::Global)];
    for agent in Agent::all() {
        roots.extend(agent.skills_dir(env, &Scope::Global));
        roots.extend(agent.native_global_dir(env));
    }
    roots
}

/// Refuses paths that aren't a direct child of a managed skills folder.
fn ensure_managed(env: &Env, path: &Path) -> Result<()> {
    let ok = path.parent().is_some_and(|parent| {
        managed_roots(env).iter().any(|r| r == parent)
            && path
                .file_name()
                .is_some_and(|n| !n.is_empty() && n != "." && n != "..")
    });
    if ok {
        Ok(())
    } else {
        Err(Error::PathEscape {
            path: path.to_path_buf(),
            base: canonical_skills_dir(env, &Scope::Global),
        })
    }
}

fn is_link_to(link: &Path, target: &Path) -> bool {
    std::fs::read_link(link).is_ok_and(|t| t == target)
}

/// Links `link` to the folder `target`: a symlink on Unix. On Windows a
/// directory symlink needs Developer Mode, so fall back to a copy, as the CLI
/// does when symlinks fail.
fn link_dir(target: &Path, link: &Path) -> Result<()> {
    match link_raw(target, link) {
        Ok(()) => Ok(()),
        Err(_) if cfg!(windows) => copy_dir(target, link),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn link_raw(target: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link).map_err(|e| Error::io(link, e))
}

#[cfg(windows)]
fn link_raw(target: &Path, link: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(target, link).map_err(|e| Error::io(link, e))
}

/// Moves a folder, copying across filesystems when a rename can't.
fn move_dir(from: &Path, to: &Path) -> Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_dir(from, to)?;
            std::fs::remove_dir_all(from).map_err(|e| Error::io(from, e))
        }
        Err(e) => Err(Error::io(from, e)),
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| Error::io(to, e))?;
    for entry in std::fs::read_dir(from)
        .map_err(|e| Error::io(from, e))?
        .flatten()
    {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let meta = src.symlink_metadata().map_err(|e| Error::io(&src, e))?;
        if meta.is_dir() {
            copy_dir(&src, &dst)?;
        } else if meta.is_file() {
            std::fs::copy(&src, &dst).map_err(|e| Error::io(&src, e))?;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::scan::scan_scope;

    fn env(home: &Path) -> Env {
        Env::new(home.to_path_buf(), [])
    }

    fn files(version: &str) -> FileMap {
        let mut f = FileMap::new();
        f.insert(
            "SKILL.md".into(),
            format!("---\nname: pdf\ndescription: PDF tools {version}. Use when working with PDFs.\n---\n# PDF\n").into_bytes(),
        );
        f.insert("references/forms.md".into(), version.as_bytes().to_vec());
        f
    }

    fn agents(ids: &[&str]) -> Vec<Agent> {
        ids.iter().filter_map(|id| Agent::by_id(id)).collect()
    }

    fn lock_names(e: &Env) -> Vec<String> {
        match read_global(e).expect("read") {
            crate::lock::LockState::Loaded(l) => l.skills.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn install_links_records_and_undoes() {
        let home = tempfile::tempdir().expect("tempdir");
        let e = env(home.path());
        let out = install(
            &e,
            "acme/skills",
            &files("v1"),
            &agents(&["claude-code", "codex"]),
        )
        .expect("install");
        assert_eq!(out.linked, vec!["claude-code", "codex"]);

        let canonical = home.path().join(".agents/skills/pdf");
        let link = home.path().join(".claude/skills/pdf");
        assert!(canonical.join("references/forms.md").is_file());
        assert_eq!(std::fs::read_link(&link).expect("link"), canonical);
        assert_eq!(lock_names(&e), vec!["pdf"]);
        assert!(load_baselines(&e).contains_key("pdf"));

        // The scanner sees it for both agents.
        let report = scan_scope(&e, &Scope::Global);
        assert_eq!(report.skills.len(), 1);

        undo(&e, &out.backup_id).expect("undo");
        assert!(!canonical.exists());
        assert!(link.symlink_metadata().is_err());
        assert!(lock_names(&e).is_empty());
        assert!(load_baselines(&e).is_empty());
    }

    #[test]
    fn update_backs_up_old_version_and_undo_restores_it() {
        let home = tempfile::tempdir().expect("tempdir");
        let e = env(home.path());
        install(&e, "acme/skills", &files("v1"), &agents(&["claude-code"])).expect("v1");
        let out = install(&e, "acme/skills", &files("v2"), &agents(&["claude-code"])).expect("v2");
        let forms = home.path().join(".agents/skills/pdf/references/forms.md");
        assert_eq!(std::fs::read_to_string(&forms).expect("read"), "v2");

        undo(&e, &out.backup_id).expect("undo");
        assert_eq!(std::fs::read_to_string(&forms).expect("read"), "v1");
        // The link survives (it already pointed at the right place).
        assert!(home
            .path()
            .join(".claude/skills/pdf")
            .symlink_metadata()
            .is_ok());
        assert_eq!(lock_names(&e), vec!["pdf"]);
    }

    #[test]
    fn remove_moves_everything_to_backup_and_undo_restores() {
        let home = tempfile::tempdir().expect("tempdir");
        let e = env(home.path());
        install(&e, "acme/skills", &files("v1"), &agents(&["claude-code"])).expect("install");
        let skill = scan_scope(&e, &Scope::Global).skills.remove(0);

        let out = remove(&e, &skill).expect("remove");
        let canonical = home.path().join(".agents/skills/pdf");
        assert!(!canonical.exists());
        assert!(home
            .path()
            .join(".claude/skills/pdf")
            .symlink_metadata()
            .is_err());
        assert!(lock_names(&e).is_empty());
        assert_eq!(list_backups(&e)[0].action, "remove");

        undo(&e, &out.backup_id).expect("undo");
        assert!(canonical.join("SKILL.md").is_file());
        assert!(home
            .path()
            .join(".claude/skills/pdf")
            .symlink_metadata()
            .is_ok());
        assert_eq!(lock_names(&e), vec!["pdf"]);
    }

    #[test]
    fn fixes_broken_links_and_drifted_copies() {
        let home = tempfile::tempdir().expect("tempdir");
        let e = env(home.path());
        let skills = home.path().join(".claude/skills");
        std::fs::create_dir_all(&skills).expect("mkdir");
        let dead = skills.join("gone");
        std::os::unix::fs::symlink(home.path().join("nowhere"), &dead).expect("symlink");
        let out = remove_broken_link(&e, &dead).expect("fix");
        assert!(dead.symlink_metadata().is_err());
        undo(&e, &out.backup_id).expect("undo");
        assert!(dead.symlink_metadata().is_ok());

        install(&e, "acme/skills", &files("v1"), &[]).expect("install");
        let canonical = home.path().join(".agents/skills/pdf");
        let copy = skills.join("pdf");
        std::fs::create_dir_all(&copy).expect("mkdir");
        std::fs::write(copy.join("SKILL.md"), "old").expect("write");
        relink_copy(&e, &copy, &canonical, "pdf").expect("relink");
        assert_eq!(std::fs::read_link(&copy).expect("link"), canonical);
    }

    #[test]
    fn removes_broken_links_together_and_undo_restores_all() {
        let home = tempfile::tempdir().expect("tempdir");
        let e = env(home.path());
        let mut dead = Vec::new();
        for dir in [".claude/skills", ".cursor/skills"] {
            let skills = home.path().join(dir);
            std::fs::create_dir_all(&skills).expect("mkdir");
            for name in ["gone", "lost"] {
                let link = skills.join(name);
                std::os::unix::fs::symlink(home.path().join("nowhere"), &link).expect("symlink");
                dead.push(link);
            }
        }
        // A link that works again and a path outside skill folders are skipped
        // or refused, never removed.
        install(&e, "acme/skills", &files("v1"), &agents(&["claude-code"])).expect("install");
        let working = home.path().join(".claude/skills/pdf");
        let mut paths = dead.clone();
        paths.push(working.clone());

        let out = remove_broken_links(&e, &paths).expect("fix all");
        assert_eq!(out.skill, "4 broken links");
        assert!(dead.iter().all(|p| p.symlink_metadata().is_err()));
        assert!(working.join("SKILL.md").is_file());

        undo(&e, &out.backup_id).expect("undo");
        assert!(dead.iter().all(|p| p.symlink_metadata().is_ok()));

        let outside = home.path().join("Documents/link");
        std::fs::create_dir_all(home.path().join("Documents")).expect("mkdir");
        std::os::unix::fs::symlink(home.path().join("nowhere"), &outside).expect("symlink");
        assert!(remove_broken_links(&e, &[dead[0].clone(), outside.clone()]).is_err());
        // The failed run rolled back: the first link is restored.
        assert!(dead[0].symlink_metadata().is_ok());
        assert!(outside.symlink_metadata().is_ok());
        assert!(remove_broken_links(&e, &[working]).is_err());
    }

    #[test]
    fn refuses_paths_outside_skill_folders() {
        let home = tempfile::tempdir().expect("tempdir");
        let e = env(home.path());
        let outside = home.path().join("Documents");
        std::fs::create_dir_all(&outside).expect("mkdir");
        let mut b = Backup::new(&e, "fix", "x").expect("backup");
        assert!(matches!(b.stash(&outside), Err(Error::PathEscape { .. })));
        assert!(outside.exists());
        assert!(undo(&e, "../etc").is_err());
    }

    #[test]
    fn edit_validates_backs_up_and_undoes() {
        let home = tempfile::tempdir().expect("tempdir");
        let e = env(home.path());
        install(&e, "acme/skills", &files("v1"), &agents(&["claude-code"])).expect("install");
        let skill = scan_scope(&e, &Scope::Global).skills.remove(0);
        let md = home.path().join(".agents/skills/pdf/SKILL.md");
        let original = std::fs::read_to_string(&md).expect("read");

        // Renames and broken frontmatter are refused, and nothing changes.
        assert!(edit_skill_md(&e, &skill, "---\nname: other\ndescription: x\n---\n").is_err());
        assert!(edit_skill_md(&e, &skill, "no frontmatter").is_err());
        assert_eq!(std::fs::read_to_string(&md).expect("read"), original);

        let edited = "---\nname: pdf\ndescription: Edited. Use when testing.\n---\n# Mine\n";
        let out = edit_skill_md(&e, &skill, edited).expect("edit");
        assert_eq!(std::fs::read_to_string(&md).expect("read"), edited);
        // Other files are untouched.
        assert!(home
            .path()
            .join(".agents/skills/pdf/references/forms.md")
            .is_file());
        // The edit now counts as a local change.
        let local =
            crate::files::read_skill_files(&home.path().join(".agents/skills/pdf")).expect("read");
        let base = load_baselines(&e).get("pdf").cloned();
        assert_eq!(
            compare(&local, &files("v1"), base.as_deref()),
            UpdateState::EditedLocally
        );

        undo(&e, &out.backup_id).expect("undo");
        assert_eq!(std::fs::read_to_string(&md).expect("read"), original);
        assert!(home
            .path()
            .join(".claude/skills/pdf")
            .symlink_metadata()
            .is_ok());
    }

    #[test]
    fn compare_uses_baseline() {
        let (v1, v2) = (files("v1"), files("v2"));
        let base = content_hash(&v1);
        assert_eq!(compare(&v1, &v1, None), UpdateState::UpToDate);
        assert_eq!(compare(&v1, &v2, None), UpdateState::Differs);
        assert_eq!(compare(&v1, &v2, Some(&base)), UpdateState::UpdateAvailable);
        assert_eq!(compare(&v2, &v1, Some(&base)), UpdateState::EditedLocally);
        assert_eq!(
            compare(&files("x"), &v2, Some(&base)),
            UpdateState::Diverged
        );
    }
}
