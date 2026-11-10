//! Defences for untrusted input: skill names become directory names, and
//! names/descriptions from SKILL.md or the skills.sh API end up on screen.

use std::path::{Component, Path};

/// Converts a skill name into a safe directory name.
///
/// Port of the CLI's `sanitizeName`, so we pick the same directory the CLI
/// would: lowercase, runs of anything outside `[a-z0-9._]` become `-`, leading
/// and trailing dots/hyphens are trimmed, at most 255 chars.
pub fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    let trimmed = out.trim_matches(|c| c == '.' || c == '-');
    // JS `substring(0, 255)` counts UTF-16 units, but the result is pure ASCII
    // here, so bytes == chars == units.
    let truncated = &trimmed[..trimmed.len().min(255)];
    if truncated.is_empty() {
        "unnamed-skill".to_owned()
    } else {
        truncated.to_owned()
    }
}

/// Makes untrusted metadata safe and tidy to display: removes control
/// characters and Unicode bidi overrides (which can make text render
/// misleadingly), collapses line breaks, and trims.
pub fn sanitize_display(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\n' || c == '\r' {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else if c == '\t' {
            out.push(' ');
        } else if !c.is_control() && !is_bidi_control(c) {
            out.push(c);
        }
    }
    out.trim().to_owned()
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// Whether `path` is `base` or lies inside it, judged lexically.
///
/// Both paths must be absolute and free of `..` for a meaningful answer; we
/// reject `..` outright rather than normalising it away.
pub fn is_within(base: &Path, path: &Path) -> bool {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    path.starts_with(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_name_matches_cli() {
        assert_eq!(
            sanitize_name("Convex Best Practices"),
            "convex-best-practices"
        );
        assert_eq!(sanitize_name("../../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize_name(".hidden"), "hidden");
        assert_eq!(sanitize_name("a__b..c"), "a__b..c");
        assert_eq!(sanitize_name("!!!"), "unnamed-skill");
        assert_eq!(sanitize_name("Ünïcode Skill"), "n-code-skill");
        assert_eq!(sanitize_name(&"x".repeat(300)).len(), 255);
    }

    #[test]
    fn sanitize_display_strips_controls_and_bidi() {
        assert_eq!(sanitize_display("  a\x1b[31mred\x07\n\nb  "), "a[31mred b");
        assert_eq!(sanitize_display("safe\u{202E}txt.exe"), "safetxt.exe");
    }

    #[test]
    fn is_within_rejects_parent_components() {
        let base = Path::new("/a/skills");
        assert!(is_within(base, Path::new("/a/skills/x")));
        assert!(!is_within(base, Path::new("/a/skills/../x")));
        assert!(!is_within(base, Path::new("/a/skills-evil/x")));
    }
}
