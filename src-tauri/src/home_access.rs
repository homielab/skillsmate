//! Home folder access for the sandboxed Mac App Store build.
//!
//! A sandboxed app can't read other apps' folders such as `~/.claude/skills`,
//! and its `$HOME` points at its own container. The user grants access once by
//! choosing their home folder in an open panel; a security-scoped bookmark
//! keeps that access across launches. The folder they choose is also the home
//! SkillsMate works against.

// Two Objective-C calls are marked unsafe by objc2; see the SAFETY notes.
#![allow(unsafe_code)]

use std::path::{Path, PathBuf};

use objc2::runtime::Bool;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{
    NSData, NSString, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL,
};

/// The user's real home folder, derived from the sandbox container path
/// (`<home>/Library/Containers/<bundle id>/Data`). Outside a sandbox `$HOME`
/// already is the real home.
pub fn real_home() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let text = home.to_str()?;
    match text.find("/Library/Containers/") {
        Some(i) => Some(PathBuf::from(&text[..i])),
        None => Some(home),
    }
}

/// The app's own data folder inside its container.
pub fn container_data_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/SkillsMate"))
}

fn bookmark_file() -> Option<PathBuf> {
    Some(container_data_dir()?.join("home-access.bookmark"))
}

/// Restores access granted on an earlier launch. Returns the home folder, or
/// `None` when the user needs to grant access (first launch, or the grant no
/// longer resolves).
pub fn restore() -> Option<PathBuf> {
    let file = bookmark_file()?;
    let bytes = std::fs::read(&file).ok()?;
    let data = NSData::with_bytes(&bytes);
    let mut stale = Bool::NO;
    // SAFETY: `stale` is a valid, writable `Bool` for the duration of the call.
    let url = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &data,
            NSURLBookmarkResolutionOptions::WithSecurityScope,
            None,
            &mut stale,
        )
    }
    .ok()?;
    let path = start_access(&url)?;
    if stale.as_bool() {
        // The folder moved or was renamed; save a fresh bookmark for next time.
        let _ = save_bookmark(&url);
    }
    Some(path)
}

/// Asks the user to choose their home folder. Returns it once access is
/// granted and saved, or an error message to show.
pub fn choose(expected: &Path) -> Result<PathBuf, String> {
    let mtm = MainThreadMarker::new().ok_or("The folder picker must run on the main thread.")?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(false);
    panel.setShowsHiddenFiles(true);
    panel.setMessage(Some(&NSString::from_str(
        "Choose your home folder so SkillsMate can manage your agents' skills.",
    )));
    panel.setPrompt(Some(&NSString::from_str("Allow Access")));
    let start =
        NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&expected.to_string_lossy()), true);
    panel.setDirectoryURL(Some(&start));

    if panel.runModal() != NSModalResponseOK {
        return Err(String::new());
    }
    let url = panel.URL().ok_or("No folder was chosen.")?;
    let chosen = url
        .path()
        .map(|p| PathBuf::from(p.to_string()))
        .ok_or("That folder can't be used.")?;
    if chosen != expected {
        return Err(format!(
            "Choose your home folder, {}, not {}.",
            expected.display(),
            chosen.display()
        ));
    }
    let path = start_access(&url).ok_or("macOS didn't grant access to that folder.")?;
    save_bookmark(&url)
        .map_err(|e| format!("Couldn't remember access to your home folder: {e}"))?;
    Ok(path)
}

fn start_access(url: &NSURL) -> Option<PathBuf> {
    // SAFETY: plain message send on a valid file URL. Access stays on for the
    // life of the process, which is what the app needs.
    if !unsafe { url.startAccessingSecurityScopedResource() } {
        return None;
    }
    url.path().map(|p| PathBuf::from(p.to_string()))
}

fn save_bookmark(url: &NSURL) -> std::io::Result<()> {
    let data = url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::WithSecurityScope,
            None,
            None,
        )
        .map_err(|e| std::io::Error::other(e.localizedDescription().to_string()))?;
    let file = bookmark_file().ok_or_else(|| std::io::Error::other("no data folder"))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, data.to_vec())
}
