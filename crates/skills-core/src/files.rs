//! Skill folder contents: reading them from disk, validating downloaded
//! snapshots, hashing, and diffing two versions.
//!
//! The exclusions match the CLI's `copyDirectory` / snapshot rules: `.git`,
//! `__pycache__` and `__pypackages__` folders and `metadata.json` files are not
//! part of a skill.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

const EXCLUDED_DIRS: [&str; 3] = [".git", "__pycache__", "__pypackages__"];
const EXCLUDED_FILES: [&str; 1] = ["metadata.json"];

/// Limits for one skill. Real skills are a handful of small files.
pub const MAX_FILES: usize = 500;
pub const MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024;
const MAX_DEPTH: usize = 12;

/// A skill's files: relative `/`-separated path → contents, sorted by path.
pub type FileMap = BTreeMap<String, Vec<u8>>;

/// Reads every file in a skill folder (no symlinks followed).
pub fn read_skill_files(dir: &Path) -> Result<FileMap> {
    let mut out = FileMap::new();
    let mut total = 0u64;
    walk(dir, dir, 0, &mut out, &mut total)?;
    Ok(out)
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut FileMap, total: &mut u64) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        if meta.is_dir() {
            if depth < MAX_DEPTH && !EXCLUDED_DIRS.contains(&name.as_str()) {
                walk(root, &path, depth + 1, out, total)?;
            }
        } else if meta.is_file() && !EXCLUDED_FILES.contains(&name.as_str()) {
            *total += meta.len();
            if out.len() >= MAX_FILES || *total > MAX_TOTAL_BYTES {
                return Err(Error::Parse(format!(
                    "skill folder {} is larger than the limit",
                    root.display()
                )));
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|_| Error::Parse("path outside skill folder".into()))?;
            let key = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
            out.insert(key, bytes);
        }
    }
    Ok(())
}

/// Checks a relative path from a downloaded snapshot before it touches disk:
/// no absolute paths, no `..`, no empty or hidden-traversal components, no
/// backslashes or NULs, and not an excluded file.
pub fn validate_snapshot_path(path: &str) -> Result<()> {
    let bad = |why: &str| Err(Error::Parse(format!("unsafe file path {path:?}: {why}")));
    if path.is_empty() || path.len() > 512 {
        return bad("empty or too long");
    }
    if path.contains('\\') || path.contains('\0') || path.contains(':') {
        return bad("invalid character");
    }
    if path.starts_with('/') {
        return bad("absolute");
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return bad("empty, '.' or '..' segment");
        }
    }
    if Path::new(path)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return bad("not a plain relative path");
    }
    Ok(())
}

/// Whether a snapshot path should be written (mirrors the CLI's exclusions).
pub fn is_installable(path: &str) -> bool {
    let mut parts: Vec<&str> = path.split('/').collect();
    let Some(file) = parts.pop() else {
        return false;
    };
    !EXCLUDED_FILES.contains(&file) && parts.iter().all(|p| !EXCLUDED_DIRS.contains(p))
}

/// SHA-256 over sorted `(path, contents)` pairs. Used for SkillsMate's own
/// baselines; it is not the CLI's `skillFolderHash` (a GitHub tree SHA).
pub fn content_hash(files: &FileMap) -> String {
    let mut h = Sha256::new();
    for (path, bytes) in files {
        h.update((path.len() as u64).to_le_bytes());
        h.update(path.as_bytes());
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// Files that look like code an agent can run.
pub fn script_files(files: &FileMap) -> Vec<String> {
    const EXT: [&str; 10] = [
        ".sh", ".bash", ".zsh", ".py", ".js", ".mjs", ".ts", ".rb", ".ps1", ".bat",
    ];
    files
        .keys()
        .filter(|p| p.starts_with("scripts/") || EXT.iter().any(|e| p.ends_with(e)))
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLineKind {
    /// A file header such as `SKILL.md (modified)`.
    File,
    Added,
    Removed,
    Context,
    /// "… 12 unchanged lines" and similar notes.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
    /// Lines added / removed across text files.
    pub lines_added: usize,
    pub lines_removed: usize,
    /// A readable diff, capped at `max_lines`.
    pub lines: Vec<DiffLine>,
}

impl FileDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    /// "2 files changed, +14 −3".
    pub fn summary(&self) -> String {
        let files = self.added.len() + self.removed.len() + self.modified.len();
        let noun = if files == 1 { "file" } else { "files" };
        format!(
            "{files} {noun} changed, +{} −{}",
            self.lines_added, self.lines_removed
        )
    }
}

/// Compares `old` (installed) with `new` (published).
pub fn diff(old: &FileMap, new: &FileMap, max_lines: usize) -> FileDiff {
    let mut d = FileDiff::default();
    let push = |d: &mut FileDiff, kind: DiffLineKind, text: String| {
        if d.lines.len() < max_lines {
            d.lines.push(DiffLine { kind, text });
        }
    };
    for path in old
        .keys()
        .chain(new.keys().filter(|k| !old.contains_key(*k)))
    {
        let (a, b) = (old.get(path), new.get(path));
        if a == b {
            continue;
        }
        let a_text = a.map(|x| String::from_utf8_lossy(x).into_owned());
        let b_text = b.map(|x| String::from_utf8_lossy(x).into_owned());
        let status = match (a, b) {
            (None, Some(_)) => {
                d.added.push(path.clone());
                "added"
            }
            (Some(_), None) => {
                d.removed.push(path.clone());
                "removed"
            }
            _ => {
                d.modified.push(path.clone());
                "modified"
            }
        };
        push(&mut d, DiffLineKind::File, format!("{path} ({status})"));
        let is_text = |x: Option<&Vec<u8>>| x.is_none_or(|v| std::str::from_utf8(v).is_ok());
        if !is_text(a) || !is_text(b) {
            push(&mut d, DiffLineKind::Note, "binary file".into());
            continue;
        }
        let (a_text, b_text) = (a_text.unwrap_or_default(), b_text.unwrap_or_default());
        let text_diff = similar::TextDiff::from_lines(&a_text, &b_text);
        for group in text_diff.grouped_ops(2) {
            for op in group {
                for change in text_diff.iter_changes(&op) {
                    let line = change.value().trim_end_matches(['\n', '\r']).to_owned();
                    let kind = match change.tag() {
                        similar::ChangeTag::Insert => {
                            d.lines_added += 1;
                            DiffLineKind::Added
                        }
                        similar::ChangeTag::Delete => {
                            d.lines_removed += 1;
                            DiffLineKind::Removed
                        }
                        similar::ChangeTag::Equal => DiffLineKind::Context,
                    };
                    push(&mut d, kind, line);
                }
            }
            push(&mut d, DiffLineKind::Note, "…".into());
        }
    }
    if d.lines.len() >= max_lines {
        d.lines.push(DiffLine {
            kind: DiffLineKind::Note,
            text: "Diff truncated.".into(),
        });
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(items: &[(&str, &str)]) -> FileMap {
        items
            .iter()
            .map(|(p, c)| ((*p).to_owned(), c.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn reads_folder_with_exclusions() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("SKILL.md"), "x").expect("write");
        std::fs::write(dir.path().join("metadata.json"), "{}").expect("write");
        std::fs::create_dir_all(dir.path().join("scripts")).expect("mkdir");
        std::fs::write(dir.path().join("scripts/run.sh"), "echo").expect("write");
        std::fs::create_dir_all(dir.path().join(".git")).expect("mkdir");
        std::fs::write(dir.path().join(".git/HEAD"), "ref").expect("write");
        let files = read_skill_files(dir.path()).expect("read");
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            vec!["SKILL.md", "scripts/run.sh"]
        );
        assert_eq!(script_files(&files), vec!["scripts/run.sh"]);
    }

    #[test]
    fn snapshot_paths_are_validated() {
        for ok in ["SKILL.md", "references/a.md", "scripts/x.py"] {
            assert!(validate_snapshot_path(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a//b",
            "a\\b",
            "C:x",
            "./a",
        ] {
            assert!(validate_snapshot_path(bad).is_err(), "{bad}");
        }
        assert!(!is_installable("metadata.json"));
        assert!(!is_installable("__pycache__/x.pyc"));
        assert!(is_installable("references/metadata.md"));
    }

    #[test]
    fn hash_is_order_independent_and_content_sensitive() {
        let a = map(&[("a", "1"), ("b", "2")]);
        let b = map(&[("b", "2"), ("a", "1")]);
        assert_eq!(content_hash(&a), content_hash(&b));
        assert_ne!(
            content_hash(&a),
            content_hash(&map(&[("a", "1"), ("b", "3")]))
        );
        // Path/content boundaries are length-prefixed.
        assert_ne!(
            content_hash(&map(&[("ab", "c")])),
            content_hash(&map(&[("a", "bc")]))
        );
    }

    #[test]
    fn diff_reports_files_and_lines() {
        let old = map(&[("SKILL.md", "a\nb\nc\n"), ("old.md", "x")]);
        let new = map(&[("SKILL.md", "a\nB\nc\nd\n"), ("new.md", "y")]);
        let d = diff(&old, &new, 100);
        assert_eq!(d.modified, vec!["SKILL.md"]);
        assert_eq!(d.removed, vec!["old.md"]);
        assert_eq!(d.added, vec!["new.md"]);
        assert_eq!((d.lines_added, d.lines_removed), (3, 2));
        assert!(d
            .lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Added && l.text == "B"));
        assert_eq!(d.summary(), "3 files changed, +3 −2");
        assert!(diff(&old, &old, 100).is_empty());
    }
}
