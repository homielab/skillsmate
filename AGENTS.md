# SkillsMate: notes for coding agents

Desktop GUI for skills.sh in Rust + Slint. See README.md for layout and commands.

## Rules
- `skills-core` has no UI dependencies and `#![forbid(unsafe_code)]`. All skill logic goes there, with tests.
- `skills-app` only wires UI to core. Blocking or network work goes to the tokio runtime (`spawn_blocking` / `spawn`); results come back via `upgrade_in_event_loop` or `slint::spawn_local`. Never block the UI thread.
- Match the upstream CLI (vercel-labs/skills) for paths, lockfile formats and naming. Check its source before changing skills.sh behavior; don't guess.
- Agent paths use `$XDG_CONFIG_HOME` or `~/.config` on all platforms (like upstream's `xdg-basedir`), not the OS config dir. Use `dirs` only for SkillsMate's own data.
- Never overwrite a lockfile that failed to parse or has another schema version (`LockState::into_writable`).
- Treat SKILL.md content and API data as untrusted: size limits, `sanitize_display`, `sanitize_name`, `is_within`.
- Back up before any destructive operation (remove, update, deactivate).
- Before finishing: `cargo fmt --all`, `cargo clippy --all-targets` (no warnings), `cargo test --workspace`.
