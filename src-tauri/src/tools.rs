// Detect, launch, and place tools. macOS only for now; other targets compile
// and report nothing installed.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::config;

const CLAUDE_APP: &str = "/Applications/Claude.app";
const CLAUDE_BIN: &str = "/Applications/Claude.app/Contents/MacOS/Claude";
const CLAUDE_BUNDLE: &str = "com.anthropic.claudefordesktop";
const CLAUDE_PROCESS: &str = "Claude";
/// Claude Desktop runs the launcher binary under this name to fetch the key.
pub const HELPER_NAME: &str = "claude-key-helper";
const PLACE_HELP: &str = "Claude opened but could not be placed in the launcher. Allow Consus Launcher under System Settings, Privacy and Security, Accessibility.";

/// PIDs of apps this launcher started. Quit when the launcher exits.
pub struct Children(pub Mutex<Vec<u32>>);

#[derive(Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

fn claude_installed(app: &AppHandle) -> bool {
    if Path::new(CLAUDE_APP).exists() {
        return true;
    }
    app.path()
        .home_dir()
        .map(|h| h.join("Applications/Claude.app").exists())
        .unwrap_or(false)
}

#[tauri::command]
pub fn detect_tools(app: AppHandle) -> HashMap<String, bool> {
    HashMap::from([("desktop".to_string(), cfg!(target_os = "macos") && claude_installed(&app))])
}

fn osascript(script: &str) -> Result<String, String> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn running_pids() -> Vec<u32> {
    Command::new("pgrep")
        .args(["-x", CLAUDE_PROCESS])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ask_claude_to_quit() {
    let _ = osascript(&format!("tell application id \"{CLAUDE_BUNDLE}\" to quit"));
}

fn quit_and_wait() -> Result<(), String> {
    ask_claude_to_quit();
    for _ in 0..40 {
        if running_pids().is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err("Claude Desktop is still running. Close it and try again.".into())
}

fn place_window(rect: &Rect) -> Result<(), String> {
    let (x, y, w, h) = (rect.x.round(), rect.y.round(), rect.w.round(), rect.h.round());
    let probe = format!(
        "tell application \"System Events\" to (exists window 1 of process \"{CLAUDE_PROCESS}\")"
    );
    let mut seen = false;
    for _ in 0..60 {
        match osascript(&probe) {
            Ok(s) if s == "true" => {
                seen = true;
                break;
            }
            Err(e) if e.contains("assistive access") => return Err(e),
            _ => thread::sleep(Duration::from_millis(250)),
        }
    }
    if !seen {
        return Err("Claude did not open a window in time.".into());
    }
    osascript(&format!(
        "tell application \"System Events\" to tell process \"{CLAUDE_PROCESS}\"\n\
           set position of window 1 to {{{x}, {y}}}\n\
           set size of window 1 to {{{w}, {h}}}\n\
         end tell"
    ))?;
    let _ = osascript(&format!("tell application id \"{CLAUDE_BUNDLE}\" to activate"));
    Ok(())
}

fn notice(app: &AppHandle, err: &str) {
    let message = if err.contains("assistive access") {
        PLACE_HELP.to_string()
    } else {
        format!("Claude opened but could not be placed: {err}")
    };
    let _ = app.emit("tool-notice", json!({ "tool": "desktop", "message": message }));
}

/// While a tool is running the launcher is a backdrop: the tool's window
/// stays above it and takes clicks, and the launcher panel still works.
fn set_backdrop(app: &AppHandle, on: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_always_on_bottom(on);
    }
}

#[tauri::command]
pub async fn launch_claude_desktop(app: AppHandle, rect: Rect, models: Value) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("Claude Desktop launch is macOS only for now".into());
    }
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let helper = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join(HELPER_NAME);
    config::write_helper(&helper)?;
    config::write_claude_desktop(&home, &helper, &models)?;

    let ours = app.state::<Children>().0.lock().unwrap().clone();
    let running = running_pids();
    if running.iter().any(|p| ours.contains(p)) {
        let handle = app.clone();
        thread::spawn(move || {
            if let Err(e) = place_window(&rect) {
                notice(&handle, &e);
            }
        });
        return Ok(());
    }
    if !running.is_empty() {
        quit_and_wait()?;
    }

    let mut child = Command::new(CLAUDE_BIN).spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    app.state::<Children>().0.lock().unwrap().push(pid);
    set_backdrop(&app, true);

    let handle = app.clone();
    thread::spawn(move || {
        if let Err(e) = place_window(&rect) {
            notice(&handle, &e);
        }
        let _ = child.wait();
        handle.state::<Children>().0.lock().unwrap().retain(|p| *p != pid);
        set_backdrop(&handle, false);
        let _ = handle.emit("tool-exited", "desktop");
    });
    Ok(())
}

#[tauri::command]
pub fn remove_claude_desktop_config(app: AppHandle) -> Result<(), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    config::remove_claude_desktop(&home)
}

/// On launcher exit the apps it started are asked to quit, then killed if
/// they ignore that. Apps the user opened themselves are left alone.
pub fn terminate_children(app: &AppHandle) {
    let ours: Vec<u32> = app
        .state::<Children>()
        .0
        .lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|p| alive(*p))
        .collect();
    if ours.is_empty() {
        return;
    }
    ask_claude_to_quit();
    for _ in 0..15 {
        if !ours.iter().any(|p| alive(*p)) {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
    for pid in ours {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}
