# SkillsMate: notes for coding agents

Desktop GUI for skills.sh: a Tauri 2 app with a Rust core and a React + TypeScript frontend. See README.md for layout and commands.

## Rules
- `crates/skills-core` has no UI dependencies and `#![forbid(unsafe_code)]`. All skill logic goes there, with tests.
- `src-tauri` only turns core results into what the UI shows, as Tauri commands. Every string, count and label is built there, so the frontend never re-derives them. Filesystem work goes through `state::blocking`; never hold the `State` lock across an `.await`.
- `src` (React) only renders and keeps view state (the open page, filters, the open dialog). It calls commands through `src/api.ts`, which mirrors the Rust types: change both together.
- Anything that changes files is prepared first (`prepare_*` returns the confirmation text), runs on `confirm_pending`, and can be undone. Don't add a command that changes files without that.
- Links open through the `open_url` command, which only allows skills.sh and this app's release pages. Don't add the opener plugin's permissions to the frontend.
- Match the upstream CLI (vercel-labs/skills) for paths, lockfile formats and naming. Check its source before changing skills.sh behavior; don't guess.
- Agent paths use `$XDG_CONFIG_HOME` or `~/.config` on all platforms (like upstream's `xdg-basedir`), not the OS config dir. Use `dirs` only for SkillsMate's own data.
- Never overwrite a lockfile that failed to parse or has another schema version (`LockState::into_writable`).
- Treat SKILL.md content and API data as untrusted: size limits, `sanitize_display`, `sanitize_name`, `is_within`. Render it as text, never as HTML.
- Back up before any destructive operation (remove, update, edit, fix).
- JavaScript packages with bun. Colour is for status and the primary action only; the rest of the UI is monochrome like the logo.
- Before finishing: `cargo fmt --all`, `cargo clippy --all-targets` (no warnings), `cargo test --workspace`, `bun run build`.
- To try changes without touching real skills: `env -u CLAUDE_CONFIG_DIR HOME=<temp dir> SKILLSMATE_DATA_DIR=<temp dir> bunx tauri dev`. Variables like `CLAUDE_CONFIG_DIR` and `CODEX_HOME` move an agent's folder, so unset them too.
