//! Parsing `SKILL.md` files.
//!
//! A skill is a directory containing `SKILL.md`, which starts with YAML
//! frontmatter that must have string `name` and `description` fields. These
//! rules mirror the CLI's `parseSkillMd`.

use std::io::Read;
use std::path::Path;

use serde_yaml_ng::{Mapping, Value};

use crate::error::{Error, Result};
use crate::sanitize::sanitize_display;

pub const SKILL_FILE: &str = "SKILL.md";

/// Refuse to parse SKILL.md files larger than this. Real ones are a few KB;
/// the cap bounds memory use and YAML parsing time on hostile input.
pub const MAX_SKILL_MD_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SkillMeta {
    /// Name exactly as written in the frontmatter (used for identity).
    pub name: String,
    /// Display-safe description.
    pub description: String,
    /// `metadata.internal: true` marks skills the CLI hides by default.
    pub internal: bool,
    /// `metadata.version`, when the author declares one.
    pub version: Option<String>,
    pub license: Option<String>,
    /// Environment requirements (Agent Skills spec, max 500 chars).
    pub compatibility: Option<String>,
    /// Pre-approved tools (experimental spec field), e.g. `Bash(git:*) Read`.
    pub allowed_tools: Option<String>,
    /// The markdown body after the frontmatter.
    pub body: String,
}

impl SkillMeta {
    /// Display-safe name.
    pub fn display_name(&self) -> String {
        sanitize_display(&self.name)
    }
}

/// Reads and parses `<dir>/SKILL.md`.
pub fn read_skill_dir(dir: &Path) -> Result<SkillMeta> {
    let path = dir.join(SKILL_FILE);
    let file = std::fs::File::open(&path).map_err(|e| Error::io(&path, e))?;
    let mut text = String::new();
    file.take(MAX_SKILL_MD_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| Error::io(&path, e))?;
    if text.len() as u64 > MAX_SKILL_MD_BYTES {
        return Err(invalid(&path, "file is larger than 1 MiB"));
    }
    parse(&text).map_err(|reason| invalid(&path, &reason))
}

/// Parses SKILL.md content.
pub fn parse(text: &str) -> std::result::Result<SkillMeta, String> {
    let (yaml, body) = split_frontmatter(text).ok_or("missing YAML frontmatter")?;
    let data: Value =
        serde_yaml_ng::from_str(yaml).map_err(|e| format!("YAML parse error: {e}"))?;
    let map = match data {
        Value::Mapping(m) => m,
        Value::Null => Mapping::new(),
        _ => return Err("frontmatter is not a mapping".into()),
    };

    let name = required_string(&map, "name")?;
    let description = required_string(&map, "description")?;
    let internal = map
        .get("metadata")
        .and_then(|m| m.get("internal"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let metadata = map.get("metadata");
    Ok(SkillMeta {
        name,
        description: sanitize_display(&description),
        internal,
        version: metadata
            .and_then(|m| m.get("version"))
            .and_then(scalar_string),
        license: map.get("license").and_then(scalar_string),
        compatibility: map.get("compatibility").and_then(scalar_string),
        allowed_tools: map.get("allowed-tools").and_then(scalar_string),
        body: body.to_owned(),
    })
}

/// A string or number scalar as display-safe text (`version: 1.0` parses as a
/// number in YAML).
fn scalar_string(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let s = sanitize_display(&s);
    (!s.is_empty()).then_some(s)
}

fn required_string(map: &Mapping, key: &str) -> std::result::Result<String, String> {
    match map.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
        Some(Value::String(_)) | None | Some(Value::Null) => {
            Err(format!("missing required frontmatter field: {key}"))
        }
        Some(_) => Err(format!("frontmatter field \"{key}\" must be a string")),
    }
}

/// Splits `---\n<yaml>\n---\n<body>`, tolerating a BOM and CRLF line endings.
/// Like the CLI, only plain `---` YAML frontmatter is accepted (never `---js`).
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;

    // Find the first line that is exactly `---`.
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let content = line.trim_end_matches('\n').trim_end_matches('\r');
        if content == "---" {
            let yaml = rest[..offset].trim_end_matches('\n').trim_end_matches('\r');
            return Some((yaml, &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

fn invalid(path: &Path, reason: &str) -> Error {
    Error::InvalidSkill {
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_skill() {
        let meta =
            parse("---\nname: pdf\ndescription: Work with PDFs\n---\n# PDF\nBody").expect("valid");
        assert_eq!(meta.name, "pdf");
        assert_eq!(meta.description, "Work with PDFs");
        assert_eq!(meta.body, "# PDF\nBody");
        assert!(!meta.internal);
    }

    #[test]
    fn accepts_bom_and_crlf() {
        let meta = parse("\u{FEFF}---\r\nname: a\r\ndescription: b\r\n---\r\nbody").expect("valid");
        assert_eq!(meta.name, "a");
        assert_eq!(meta.body, "body");
    }

    #[test]
    fn reads_optional_fields() {
        let meta = parse(
            "---\nname: a\ndescription: b\nlicense: MIT\nallowed-tools: Read\nmetadata:\n  version: 1.2\n---\n",
        )
        .expect("valid");
        assert_eq!(meta.version.as_deref(), Some("1.2"));
        assert_eq!(meta.license.as_deref(), Some("MIT"));
        assert_eq!(meta.allowed_tools.as_deref(), Some("Read"));
        assert_eq!(meta.compatibility, None);
    }

    #[test]
    fn detects_internal_flag() {
        let meta = parse("---\nname: a\ndescription: b\nmetadata:\n  internal: true\n---\n")
            .expect("valid");
        assert!(meta.internal);
    }

    #[test]
    fn rejects_missing_or_non_string_fields() {
        assert!(parse("---\nname: a\n---\n").is_err());
        assert!(parse("---\nname: 42\ndescription: b\n---\n").is_err());
        assert!(parse("no frontmatter").is_err());
        assert!(parse("---js\nname: a\n---\n").is_err());
    }

    #[test]
    fn description_is_sanitized() {
        let meta = parse("---\nname: a\ndescription: \"x\\u001b[2Jy\"\n---\n").expect("valid");
        assert_eq!(meta.description, "x[2Jy");
    }
}
