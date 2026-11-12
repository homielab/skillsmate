# winit 0.30.13, patched

Upstream winit 0.30.13 from crates.io with one change: on macOS,
`Window::set_blur` is a no-op, and the private CoreGraphics functions it called
(`CGSMainConnectionID`, `CGSSetWindowBackgroundBlurRadius`) are gone. The Mac App
Store rejects binaries that reference them. SkillsMate never uses blur.

Used via `[patch.crates-io]` in the workspace `Cargo.toml`. Drop it once a winit
release that Slint uses no longer references these functions.
