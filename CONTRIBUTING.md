# Contributing to SkillsMate

Bug reports, fixes and features are welcome. For anything larger than a small fix, open an issue first so we can agree on the approach.

## Setup

You need Rust 1.92 or newer ([rustup](https://rustup.rs)). On Linux, also install the windowing libraries:

```sh
sudo apt install libfontconfig1-dev libxkbcommon-dev   # Debian/Ubuntu
```

Then:

```sh
cargo run -p skills-app       # run the app
cargo test --workspace        # run the tests
```

## Project rules

- `crates/skills-core` holds all skill logic. It has no UI dependencies, forbids `unsafe`, and every change comes with tests.
- `crates/skills-app` only connects the Slint UI to the core. Filesystem and network work runs on the tokio runtime, never on the UI thread.
- Paths, lockfile formats and naming must match the official [`skills` CLI](https://github.com/vercel-labs/skills). Check its source before changing that behavior.
- Never overwrite a lockfile that failed to parse or has an unknown schema version.
- Treat SKILL.md files and skills.sh data as untrusted: limit sizes, sanitize names and displayed text, and keep paths inside skill folders.
- Back up before anything destructive (remove, update, fix), so Undo works.

[AGENTS.md](AGENTS.md) has the same rules in more detail.

## Before you open a pull request

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same checks on Linux, macOS and Windows. For UI changes, add a screenshot to the pull request. `SKILLSMATE_SNAPSHOT=<dir> cargo run -p skills-app` saves every page as an image.

To test against a throwaway setup instead of your real skills, point `HOME` and `SKILLSMATE_DATA_DIR` at a temporary folder.

## Updating the agent list

`crates/skills-core/src/agents/table.rs` is generated from the upstream CLI. Don't edit it by hand:

```sh
git clone --depth 1 https://github.com/vercel-labs/skills /tmp/skills
python3 tools/gen_agents.py /tmp/skills/src/agents.ts "$(git -C /tmp/skills rev-parse --short HEAD)" \
  > crates/skills-core/src/agents/table.rs
```

## Releasing (maintainers)

1. Bump `version` in the root `Cargo.toml` and commit.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. The Release workflow tests, builds the Linux `.deb` and AppImage and the macOS app, and publishes them with checksums.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
