//! The launcher lives in the menu bar (macOS) or the notification area
//! (Windows): an orange square while connected, grey while not, with an amber
//! dot when something in the panel wants a look. A click opens the panel at
//! the icon; it goes away when it loses focus.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Rect, WebviewWindow, WindowEvent, Wry};

const TRAY_ID: &str = "consus";
const ORANGE: [u8; 3] = [0xE6, 0x49, 0x1D];
const GREY: [u8; 3] = [0x7E, 0x74, 0x66];
const AMBER: [u8; 3] = [0xE7, 0xB2, 0x5A];
const WIDTH: f64 = 320.0;

/// Set while signed in. Until then the panel stays open when it loses focus:
/// sign-in sends the user to the browser and back.
static CONNECTED: AtomicBool = AtomicBool::new(false);

/// Coverage of a pixel by a shape, one pixel of antialiasing at its edge.
fn cover(distance_outside: f64) -> f64 {
    (0.5 - distance_outside).clamp(0.0, 1.0)
}

/// A rounded square, drawn rather than shipped, 36 px for 18 pt at 2x; with
/// `dot`, an amber dot on its top right corner.
fn square(rgb: [u8; 3], dot: bool) -> Image<'static> {
    const N: usize = 36;
    let (lo, hi, r) = (5.0_f64, 31.0_f64, 6.0_f64);
    let (dot_c, dot_r) = ((29.5_f64, 6.5_f64), 6.0_f64);
    let mut px = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let dx = (lo + r - cx).max(cx - (hi - r)).max(0.0);
            let dy = (lo + r - cy).max(cy - (hi - r)).max(0.0);
            let inside = cx >= lo && cx <= hi && cy >= lo && cy <= hi;
            let sq = if inside { cover((dx * dx + dy * dy).sqrt() - r) } else { 0.0 };
            let d = ((cx - dot_c.0).powi(2) + (cy - dot_c.1).powi(2)).sqrt();
            // A clear ring around the dot keeps it apart from the square.
            let (ring, dt) = if dot { (cover(d - dot_r - 2.0), cover(d - dot_r)) } else { (0.0, 0.0) };
            let base = sq * (1.0 - ring);
            let a = dt + base * (1.0 - dt);
            let mix = |i: usize| {
                if a == 0.0 {
                    0
                } else {
                    ((AMBER[i] as f64 * dt + rgb[i] as f64 * base * (1.0 - dt)) / a).round() as u8
                }
            };
            let i = (y * N + x) * 4;
            px[i..i + 4].copy_from_slice(&[mix(0), mix(1), mix(2), (a * 255.0).round() as u8]);
        }
    }
    Image::new_owned(px, N as u32, N as u32)
}

fn panel(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn icon_rect(app: &AppHandle) -> Option<Rect> {
    app.tray_by_id(TRAY_ID)?.rect().ok().flatten()
}

/// Puts the panel against the icon: below it in a top menu bar, above it in a
/// bottom taskbar, kept on the icon's screen.
fn place(w: &WebviewWindow, icon: &Rect) {
    let scale = w.scale_factor().unwrap_or(1.0);
    let p = icon.position.to_physical::<f64>(scale);
    let s = icon.size.to_physical::<f64>(scale);
    let Ok(size) = w.outer_size() else { return };
    let (pw, ph) = (size.width as f64, size.height as f64);
    let gap = 6.0 * scale;
    let mut x = p.x + s.width / 2.0 - pw / 2.0;
    let mut y = p.y + s.height + gap;
    if let Ok(Some(m)) = w.monitor_from_point(p.x, p.y) {
        let (mp, ms) = (m.work_area().position, m.work_area().size);
        let (left, top) = (mp.x as f64, mp.y as f64);
        let (right, bottom) = (left + ms.width as f64, top + ms.height as f64);
        if p.y > top + ms.height as f64 / 2.0 {
            y = p.y - ph - gap;
        }
        x = x.clamp(left + gap, (right - pw - gap).max(left));
        y = y.clamp(top, (bottom - ph).max(top));
    }
    let _ = w.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

pub fn show(app: &AppHandle) {
    let Some(w) = panel(app) else { return };
    match icon_rect(app) {
        Some(r) => place(&w, &r),
        None => {
            let _ = w.center();
        }
    }
    let _ = w.show();
    let _ = w.set_focus();
}

fn toggle(app: &AppHandle, icon: &Rect) {
    let Some(w) = panel(app) else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
    } else {
        place(&w, icon);
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[derive(Deserialize)]
pub struct MenuTool {
    key: String,
    name: String,
}

/// The icon's menu: the panel, each tool ready to open, a newer launcher if
/// there is one, and Quit.
fn menu(app: &AppHandle, tools: &[MenuTool], update: Option<&str>) -> tauri::Result<Menu<Wry>> {
    let m = Menu::new(app)?;
    m.append(&MenuItem::with_id(app, "open", "Open Consus Launcher", true, None::<&str>)?)?;
    if !tools.is_empty() {
        m.append(&PredefinedMenuItem::separator(app)?)?;
        for t in tools {
            m.append(&MenuItem::with_id(app, format!("tool:{}", t.key), format!("Open {}", t.name), true, None::<&str>)?)?;
        }
    }
    m.append(&PredefinedMenuItem::separator(app)?)?;
    if let Some(v) = update {
        m.append(&MenuItem::with_id(app, "update", format!("Download Consus Launcher {v}\u{2026}"), true, None::<&str>)?)?;
    }
    m.append(&MenuItem::with_id(app, "quit", "Quit Consus Launcher", true, None::<&str>)?)?;
    Ok(m)
}

pub fn build(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(square(GREY, false))
        .tooltip("Consus Launcher: not signed in")
        .menu(&menu(app, &[], None)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => show(app),
            // Quitting also quits the tools the launcher started (RunEvent::Exit).
            "quit" => app.exit(0),
            // The panel knows how to open a tool and where the download is.
            "update" => {
                let _ = app.emit("tray-update", ());
            }
            id => {
                if let Some(key) = id.strip_prefix("tool:") {
                    let _ = app.emit("tray-tool", key);
                }
            }
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = e {
                toggle(tray.app_handle(), &rect);
            }
        })
        .build(app)?;

    if let Some(w) = panel(app) {
        let handle = app.clone();
        w.on_window_event(move |e| match e {
            // Closing only hides; the menu's Quit ends the launcher.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                if let Some(w) = panel(&handle) {
                    let _ = w.hide();
                }
            }
            WindowEvent::Focused(false) if CONNECTED.load(Ordering::Relaxed) => {
                if let Some(w) = panel(&handle) {
                    let _ = w.hide();
                }
            }
            _ => {}
        });
    }
    Ok(tray)
}

/// The UI reports sign-in state and whether something wants a look; the
/// icon follows.
#[tauri::command]
pub fn set_tray_state(app: AppHandle, connected: bool, attention: bool) {
    CONNECTED.store(connected, Ordering::Relaxed);
    if let Some(t) = app.tray_by_id(TRAY_ID) {
        let _ = t.set_icon(Some(square(if connected { ORANGE } else { GREY }, attention)));
        let tip = match (connected, attention) {
            (true, false) => "Consus Launcher: connected",
            (true, true) => "Consus Launcher: connected, something needs a look",
            (false, _) => "Consus Launcher: not signed in",
        };
        let _ = t.set_tooltip(Some(tip));
    }
}

#[tauri::command]
pub fn set_tray_menu(app: AppHandle, tools: Vec<MenuTool>, update: Option<String>) -> Result<(), String> {
    let m = menu(&app, &tools, update.as_deref()).map_err(|e| e.to_string())?;
    if let Some(t) = app.tray_by_id(TRAY_ID) {
        t.set_menu(Some(m)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn show_panel(app: AppHandle) {
    show(&app);
}

/// Puts the panel away, after a tool opens.
#[tauri::command]
pub fn hide_panel(app: AppHandle) {
    if let Some(w) = panel(&app) {
        let _ = w.hide();
    }
}

/// Sizes the panel to its content and keeps it against the icon.
#[tauri::command]
pub fn fit_panel(app: AppHandle, height: f64) {
    let Some(w) = panel(&app) else { return };
    let _ = w.set_size(LogicalSize::new(WIDTH, height.clamp(240.0, 640.0)));
    if w.is_visible().unwrap_or(false) {
        if let Some(r) = icon_rect(&app) {
            place(&w, &r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(img: &Image, x: usize, y: usize) -> u8 {
        img.rgba()[(y * 36 + x) * 4 + 3]
    }

    #[test]
    fn the_square_has_rounded_corners_and_a_clear_margin() {
        let img = square(ORANGE, false);
        assert_eq!(alpha(&img, 18, 18), 255);
        assert_eq!(alpha(&img, 1, 1), 0);
        // Rounded: the very corner of the square is transparent.
        assert_eq!(alpha(&img, 5, 5), 0);
    }

    #[test]
    fn the_dot_is_amber_and_only_there_when_asked() {
        let plain = square(ORANGE, false);
        let dotted = square(ORANGE, true);
        let at = |img: &Image, x: usize, y: usize| img.rgba()[(y * 36 + x) * 4..(y * 36 + x) * 4 + 4].to_vec();
        assert_eq!(at(&dotted, 29, 6), vec![AMBER[0], AMBER[1], AMBER[2], 255]);
        assert_eq!(alpha(&plain, 32, 2), 0);
        assert_eq!(&at(&plain, 18, 18)[..3], &ORANGE);
        assert_eq!(&at(&dotted, 12, 24)[..3], &ORANGE);
    }
}
