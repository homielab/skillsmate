//! Developer tool: `SKILLSMATE_SNAPSHOT=<dir>` opens each page in turn, saves
//! the window as `<dir>/<page>.bmp`, then quits. Used to check the UI on
//! machines where screen capture isn't available (CI, remote sessions), and
//! for store screenshots: `SKILLSMATE_SNAPSHOT_SIZE=1440x900` sets the window
//! size in logical pixels.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use slint::{ComponentHandle, Model, Timer, TimerMode};

use crate::{AppWindow, Page};

const PAGES: &[(&str, Page)] = &[
    ("overview", Page::Overview),
    ("installed", Page::Installed),
    ("health", Page::Health),
    ("audits", Page::Audits),
    ("discover", Page::Discover),
    ("official", Page::Official),
    ("topics", Page::Topics),
    ("agents", Page::Agents),
    ("detail", Page::Detail),
];

/// Time each page gets to load its data before it is captured.
const SETTLE: Duration = Duration::from_secs(4);

pub fn start(ui: &AppWindow, dir: PathBuf) {
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("snapshot: can't create {}: {err}", dir.display());
        return;
    }
    if let Some((w, h)) = std::env::var("SKILLSMATE_SNAPSHOT_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?))
        })
    {
        ui.window().set_size(slint::LogicalSize::new(w, h));
    }
    let step = Rc::new(Cell::new(0usize));
    let timer = Rc::new(Timer::default());
    let weak = ui.as_weak();
    let keep = timer.clone();
    timer.start(TimerMode::Repeated, SETTLE, move || {
        let _ = &keep;
        let Some(ui) = weak.upgrade() else { return };
        let i = step.get();
        step.set(i + 1);
        // Each tick captures the page opened on the tick before, then opens
        // the next one once the capture is written.
        let next = move |ui: &AppWindow| {
            let Some(&(_, page)) = PAGES.get(i) else {
                let _ = slint::quit_event_loop();
                return;
            };
            if page == Page::Detail {
                if let Some(row) = ui.get_installed().row_data(0) {
                    ui.set_detail_back(Page::Installed);
                    ui.set_page(Page::Detail);
                    ui.invoke_open_skill(row.key);
                }
            } else {
                ui.set_page(page);
                ui.invoke_page_changed(page);
            }
        };
        match i.checked_sub(1).and_then(|prev| PAGES.get(prev)) {
            Some((name, _)) => save(&ui, &dir.join(format!("{name}.bmp")), next),
            None => next(&ui),
        }
    });
}

/// Captures the window, then runs `then`. Waits for a fresh frame first:
/// `take_snapshot` returns the last frame shown, which can lag behind changes.
fn save(ui: &AppWindow, path: &Path, then: impl FnOnce(&AppWindow) + 'static) {
    ui.window().request_redraw();
    let weak = ui.as_weak();
    let path = path.to_path_buf();
    Timer::single_shot(Duration::from_millis(300), move || {
        let Some(ui) = weak.upgrade() else { return };
        match ui.window().take_snapshot() {
            Ok(buf) => {
                let bmp = encode_bmp(buf.width(), buf.height(), buf.as_bytes());
                match std::fs::write(&path, bmp) {
                    Ok(()) => eprintln!("snapshot: {}", path.display()),
                    Err(err) => eprintln!("snapshot: can't write {}: {err}", path.display()),
                }
            }
            Err(err) => eprintln!("snapshot: capture failed: {err}"),
        }
        then(&ui);
    });
}

/// Uncompressed 32-bit top-down BMP from RGBA bytes.
fn encode_bmp(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let data_len = width as usize * height as usize * 4;
    let mut out = Vec::with_capacity(54 + data_len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(-(height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for px in rgba.as_chunks::<4>().0 {
        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    out
}
