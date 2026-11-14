# Contributing to SkillsMate

Bug reports, fixes and features are welcome. For anything larger than a small fix, open an issue first so we can agree on the approach.

## Setup

You need Rust 1.92 or newer ([rustup](https://rustup.rs)), [Bun](https://bun.sh), and Tauri's [system dependencies](https://tauri.app/start/prerequisites/). On Debian/Ubuntu:

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev librsvg2-dev
```

Then:

```sh
bun install
bunx tauri dev            # run the app, reloading on changes
cargo test --workspace    # run the tests
```

To test against a throwaway setup instead of your real skills, point `HOME` and `SKILLSMATE_DATA_DIR` at a temporary folder, and unset variables that move an agent's folder (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`).

## Project rules

- `crates/skills-core` holds all skill logic. It has no UI dependencies, forbids `unsafe`, and every change comes with tests.
- `src-tauri` exposes the core to the UI as Tauri commands and builds everything the UI shows. Filesystem and network work runs off the main thread.
- `src` is the React frontend. It renders what the commands return and keeps view state only. `src/api.ts` mirrors the Rust types.
- Paths, lockfile formats and naming must match the official [`skills` CLI](https://github.com/vercel-labs/skills). Check its source before changing that behavior.
- Never overwrite a lockfile that failed to parse or has an unknown schema version.
- Treat SKILL.md files and skills.sh data as untrusted: limit sizes, sanitize names and displayed text, keep paths inside skill folders, and render content as text.
- Back up before anything destructive (remove, update, edit, fix), so Undo works.

[AGENTS.md](AGENTS.md) has the same rules in more detail.

## Before you open a pull request

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace
bun run build
```

CI runs the same checks on Linux, macOS and Windows. For UI changes, add a screenshot to the pull request.

## Updating the agent list

`crates/skills-core/src/agents/table.rs` is generated from the upstream CLI. Don't edit it by hand:

```sh
git clone --depth 1 https://github.com/vercel-labs/skills /tmp/skills
python3 tools/gen_agents.py /tmp/skills/src/agents.ts "$(git -C /tmp/skills rev-parse --short HEAD)" \
  > crates/skills-core/src/agents/table.rs
```

## Releasing (maintainers)

1. Bump `version` in the root `Cargo.toml` and in `package.json`, and commit.
2. Tag and push: `git tag v0.2.0 && git push origin v0.2.0`.
3. The Release workflow tests, builds the Linux `.deb` and AppImage, the macOS `.dmg` and the Windows installer, and publishes them with checksums.

To build the packages for your own platform: `bunx tauri build` (output in `target/release/bundle/`).

### Mac App Store

The App Store build is sandboxed (`--features app-store`): it asks for the home folder on first launch and has no update check. It's signed by Xcode's automatic signing with an App Store Connect API key, so no certificates are needed locally.

Put the key in `fastlane/.env` (git-ignored): `ASC_KEY_ID`, `ASC_ISSUER_ID`, and `ASC_KEY_PATH` if the `.p8` file isn't in `~/.appstoreconnect/private_keys/`. Then:

```sh
fastlane mac beta      # build the universal .pkg and upload it (build number = latest + 1)
fastlane mac listing   # upload name, description, keywords, URLs and screenshots from fastlane/
fastlane mac submit    # submit the latest uploaded build for review
```

Store screenshots are 1440×900 window captures.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
