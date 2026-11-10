fn main() {
    // Pick the widget style that feels native on each platform. Both follow the
    // system light/dark setting at runtime.
    let style = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => "cupertino",
        _ => "fluent",
    };
    let config = slint_build::CompilerConfiguration::new().with_style(style.into());
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI failed to compile");
}
