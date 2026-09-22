// Detect, launch, and place tools. macOS only for now; other targets compile
// and report nothing installed.

use serde::Deserialize;
use serde_json::Value;
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

/// PIDs of apps this launcher started. Terminated when the launcher exits.
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

fn quit_and_wait() {
    let _ = osascript(&format!("tell application id \"{CLAUDE_BUNDLE}\" to quit"));
    for _ in 0..40 {
        if running_pids().is_empty() {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn place_window(rect: &Rect) {
    let (x, y, w, h) = (rect.x.round(), rect.y.round(), rect.w.round(), rect.h.round());
    for _ in 0..60 {
        let exists = osascript(&format!(
            "tell application \"System Events\" to (exists window 1 of process \"{CLAUDE_PROCESS}\")"
        ));
        if exists.as_deref() == Ok("true") {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    let _ = osascript(&format!(
        "tell application \"System Events\" to tell process \"{CLAUDE_PROCESS}\"\n\
           set position of window 1 to {{{x}, {y}}}\n\
           set size of window 1 to {{{w}, {h}}}\n\
         end tell"
    ));
    let _ = osascript(&format!("tell application id \"{CLAUDE_BUNDLE}\" to activate"));
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
        .join("claude-key-helper");
    config::write_helper(&helper)?;
    config::write_claude_desktop(&home, &helper, &models)?;

    let ours = app.state::<Children>().0.lock().unwrap().clone();
    let running = running_pids();
    if !running.is_empty() && running.iter().any(|p| ours.contains(p)) {
        place_window(&rect);
        return Ok(());
    }
    if !running.is_empty() {
        quit_and_wait();
    }

    let mut child = Command::new(CLAUDE_BIN).spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    app.state::<Children>().0.lock().unwrap().push(pid);

    let handle = app.clone();
    thread::spawn(move || {
        place_window(&rect);
        let _ = child.wait();
        handle.state::<Children>().0.lock().unwrap().retain(|p| *p != pid);
        let _ = handle.emit("tool-exited", "desktop");
    });
    Ok(())
}

#[tauri::command]
pub fn remove_claude_desktop_config(app: AppHandle) -> Result<(), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    config::remove_claude_desktop(&home)
}

/// Called on launcher exit: the apps it started go with it.
pub fn terminate_children(app: &AppHandle) {
    let pids = app.state::<Children>().0.lock().unwrap().clone();
    for pid in pids {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}
