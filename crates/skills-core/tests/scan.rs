//! Filesystem integration tests for scanning installed skills, using a fake
//! home directory laid out the way the `skills` CLI leaves it.

use std::fs;
use std::path::Path;

use skills_core::scan::{scan_scope, LinkKind};
use skills_core::{Env, Scope};

fn write_skill(dir: &Path, name: &str, description: &str) {
    fs::create_dir_all(dir).expect("mkdir");
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n"),
    )
    .expect("write SKILL.md");
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) {
    fs::create_dir_all(link.parent().expect("parent")).expect("mkdir");
    std::os::unix::fs::symlink(target, link).expect("symlink");
}

#[test]
fn empty_home_has_no_skills() {
    let home = tempfile::tempdir().expect("tempdir");
    let report = scan_scope(&Env::new(home.path().into(), []), &Scope::Global);
    assert!(report.skills.is_empty());
    assert!(report.problems.is_empty());
}

#[test]
fn canonical_skill_is_attributed_to_detected_universal_agents() {
    let home = tempfile::tempdir().expect("tempdir");
    write_skill(&home.path().join(".agents/skills/pdf"), "pdf", "PDF tools");
    // Codex is universal and detected via ~/.codex.
    fs::create_dir_all(home.path().join(".codex")).expect("mkdir");

    let report = scan_scope(&Env::new(home.path().into(), []), &Scope::Global);
    assert_eq!(report.skills.len(), 1);
    let skill = &report.skills[0];
    assert_eq!(skill.meta.name, "pdf");
    assert!(skill.canonical_path.is_some());
    assert!(skill
        .installs
        .iter()
        .any(|i| i.agent == "codex" && i.kind == LinkKind::Canonical));
}

#[cfg(unix)]
#[test]
fn symlinked_agent_install_is_merged_with_canonical() {
    let home = tempfile::tempdir().expect("tempdir");
    let canonical = home.path().join(".agents/skills/pdf");
    write_skill(&canonical, "pdf", "PDF tools");
    link_dir(&canonical, &home.path().join(".claude/skills/pdf"));

    let report = scan_scope(&Env::new(home.path().into(), []), &Scope::Global);
    assert_eq!(report.skills.len(), 1);
    let claude = report.skills[0]
        .installs
        .iter()
        .find(|i| i.agent == "claude-code")
        .expect("claude-code install");
    assert_eq!(claude.kind, LinkKind::Symlink);
}

#[test]
fn universal_agents_native_global_dir_is_scanned() {
    // Cursor installs into ~/.agents/skills but also loads ~/.cursor/skills.
    let home = tempfile::tempdir().expect("tempdir");
    write_skill(
        &home.path().join(".cursor/skills/legacy"),
        "legacy",
        "old install",
    );

    let report = scan_scope(&Env::new(home.path().into(), []), &Scope::Global);
    let skill = report
        .skills
        .iter()
        .find(|s| s.meta.name == "legacy")
        .expect("found");
    assert!(skill
        .installs
        .iter()
        .any(|i| i.agent == "cursor" && i.kind == LinkKind::Copy));
}

#[test]
fn copied_install_without_canonical_is_found() {
    let project = tempfile::tempdir().expect("tempdir");
    write_skill(
        &project.path().join(".claude/skills/review"),
        "review",
        "Code review",
    );

    let env = Env::new(tempfile::tempdir().expect("home").path().into(), []);
    let report = scan_scope(&env, &Scope::Project(project.path().into()));
    assert_eq!(report.skills.len(), 1);
    let skill = &report.skills[0];
    assert!(skill.canonical_path.is_none());
    assert_eq!(skill.installs[0].agent, "claude-code");
    assert_eq!(skill.installs[0].kind, LinkKind::Copy);
}

#[test]
fn broken_skills_are_reported_not_fatal() {
    let home = tempfile::tempdir().expect("tempdir");
    let bad = home.path().join(".agents/skills/bad");
    fs::create_dir_all(&bad).expect("mkdir");
    fs::write(bad.join("SKILL.md"), "---\nname: bad\n---\n").expect("write");
    write_skill(&home.path().join(".agents/skills/good"), "good", "ok");

    let report = scan_scope(&Env::new(home.path().into(), []), &Scope::Global);
    assert_eq!(report.skills.len(), 1);
    assert_eq!(report.problems.len(), 1);
    assert!(report.problems[0].reason.contains("description"));
}
