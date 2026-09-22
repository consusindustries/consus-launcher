// Detect, launch, and place tools. macOS only for now; other targets compile
// and report nothing installed.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

use crate::{chatgpt, config, keychain};

/// Claude Desktop runs the launcher binary under this name to fetch the key.
pub const HELPER_NAME: &str = "claude-key-helper";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Desktop,
    ChatGpt,
}

const TOOLS: [Tool; 2] = [Tool::Desktop, Tool::ChatGpt];

#[derive(Clone, Copy)]
struct Spec {
    key: &'static str,
    name: &'static str,
    app_name: &'static str,
    exec: &'static str,
    bundle: &'static str,
    process: &'static str,
}

fn spec(t: Tool) -> Spec {
    match t {
        Tool::Desktop => Spec {
            key: "desktop",
            name: "Claude",
            app_name: "Claude.app",
            exec: "Claude",
            bundle: "com.anthropic.claudefordesktop",
            process: "Claude",
        },
        Tool::ChatGpt => Spec {
            key: "chatgpt",
            name: "ChatGPT",
            app_name: "ChatGPT.app",
            exec: "ChatGPT",
            bundle: "com.openai.codex",
            process: "ChatGPT",
        },
    }
}

fn tool_from_key(key: &str) -> Option<Tool> {
    TOOLS.into_iter().find(|t| spec(*t).key == key)
}

/// (tool, pid) of apps this launcher started. Quit when the launcher exits.
pub struct Children(pub Mutex<Vec<(Tool, u32)>>);

#[derive(Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The app bundle, system-wide or per-user, whichever exists.
fn app_path(app: &AppHandle, s: &Spec) -> Option<PathBuf> {
    let system = Path::new("/Applications").join(s.app_name);
    if system.exists() {
        return Some(system);
    }
    let user = app.path().home_dir().ok()?.join("Applications").join(s.app_name);
    user.exists().then_some(user)
}

type Signature = (SystemTime, u64);

/// Per tool: the .icns the icon came from, that file's signature, and the
/// data URL, so repeat calls cost one stat instead of process spawns.
static ICONS: LazyLock<Mutex<HashMap<&'static str, (PathBuf, Signature, String)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn signature(p: &Path) -> Option<Signature> {
    let m = fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn plist_string(plist: &Path, key: &str) -> Option<String> {
    let out = Command::new("defaults").arg("read").arg(plist).arg(key).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn icns_path(bundle_dir: &Path) -> Option<PathBuf> {
    let plist = bundle_dir.join("Contents/Info.plist");
    ["CFBundleIconFile", "CFBundleIconName"].iter().find_map(|k| {
        let mut name = plist_string(&plist, k)?;
        if !name.ends_with(".icns") {
            name.push_str(".icns");
        }
        let p = bundle_dir.join("Contents/Resources").join(name);
        p.exists().then_some(p)
    })
}

/// The app's own icon as a PNG data URL, converted once from the bundle's
/// .icns. The cache file is named by the .icns's mtime and size, so an app
/// update with a new icon is picked up even when the updater preserves old
/// file dates. None on other platforms or if anything is missing.
fn app_icon(app: &AppHandle, s: &Spec, bundle_dir: &Path) -> Option<String> {
    if let Some((icns, sig, url)) = ICONS.lock().unwrap().get(s.key) {
        if icns.starts_with(bundle_dir) && signature(icns).as_ref() == Some(sig) {
            return Some(url.clone());
        }
    }
    let icns = icns_path(bundle_dir)?;
    let sig = signature(&icns)?;
    let secs = sig.0.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let dir = app.path().app_cache_dir().ok()?.join("icons");
    let png = dir.join(format!("{}-{secs}-{}.png", s.key, sig.1));
    if !png.exists() {
        fs::create_dir_all(&dir).ok()?;
        // Convert to a temp name and rename, so an interrupted run never
        // leaves a half-written file that would be served forever.
        let tmp = dir.join(format!("{}-{secs}-{}.tmp.png", s.key, sig.1));
        let ok = Command::new("sips")
            .args(["-s", "format", "png", "-Z", "128"])
            .arg(&icns)
            .arg("--out")
            .arg(&tmp)
            .output()
            .ok()?
            .status
            .success();
        if !ok || fs::rename(&tmp, &png).is_err() {
            let _ = fs::remove_file(&tmp);
            return None;
        }
    }
    let bytes = fs::read(&png).ok()?;
    let url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    ICONS.lock().unwrap().insert(s.key, (icns, sig, url.clone()));
    Some(url)
}

#[derive(Serialize)]
pub struct ToolStatus {
    pub installed: bool,
    pub icon: Option<String>,
}

#[tauri::command]
pub async fn detect_tools(app: AppHandle) -> HashMap<String, ToolStatus> {
    TOOLS
        .into_iter()
        .map(|t| {
            let s = spec(t);
            let dir = if cfg!(target_os = "macos") { app_path(&app, &s) } else { None };
            let icon = dir.as_deref().and_then(|d| app_icon(&app, &s, d));
            (s.key.to_string(), ToolStatus { installed: dir.is_some(), icon })
        })
        .collect()
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

fn running_pids(process: &str) -> Vec<u32> {
    Command::new("pgrep")
        .args(["-x", process])
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

fn ask_to_quit(bundle: &str) {
    let _ = osascript(&format!("tell application id \"{bundle}\" to quit"));
}

fn quit_and_wait(s: &Spec) -> Result<(), String> {
    ask_to_quit(s.bundle);
    for _ in 0..40 {
        if running_pids(s.process).is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(format!("{} is still running. Close it and try again.", s.name))
}

fn place_window(s: &Spec, rect: &Rect) -> Result<(), String> {
    let (x, y, w, h) = (rect.x.round(), rect.y.round(), rect.w.round(), rect.h.round());
    let probe = format!(
        "tell application \"System Events\" to (exists window 1 of process \"{}\")",
        s.process
    );
    let mut seen = false;
    for _ in 0..60 {
        match osascript(&probe) {
            Ok(v) if v == "true" => {
                seen = true;
                break;
            }
            Err(e) if e.contains("assistive access") => return Err(e),
            _ => thread::sleep(Duration::from_millis(250)),
        }
    }
    if !seen {
        return Err(format!("{} did not open a window in time.", s.name));
    }
    osascript(&format!(
        "tell application \"System Events\" to tell process \"{}\"\n\
           set position of window 1 to {{{x}, {y}}}\n\
           set size of window 1 to {{{w}, {h}}}\n\
         end tell",
        s.process
    ))?;
    let _ = osascript(&format!("tell application id \"{}\" to activate", s.bundle));
    Ok(())
}

fn notice(app: &AppHandle, s: &Spec, err: &str) {
    let message = if err.contains("assistive access") {
        format!(
            "{} opened but could not be placed in the launcher. Allow Consus Launcher under System Settings, Privacy and Security, Accessibility.",
            s.name
        )
    } else {
        format!("{} opened but could not be placed: {err}", s.name)
    };
    let _ = app.emit("tool-notice", json!({ "tool": s.key, "message": message }));
}

/// While a tool is running the launcher is a backdrop: the tool's window
/// stays above it and takes clicks, and the launcher panel still works.
fn set_backdrop(app: &AppHandle, on: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_always_on_bottom(on);
    }
}

fn our_pids(app: &AppHandle, t: Tool) -> Vec<u32> {
    let children = app.state::<Children>();
    let guard = children.0.lock().unwrap();
    guard.iter().filter(|(x, _)| *x == t).map(|(_, p)| *p).collect()
}

#[tauri::command]
pub async fn launch_tool(app: AppHandle, tool: String, rect: Rect, models: Value) -> Result<(), String> {
    let t = tool_from_key(&tool).ok_or_else(|| format!("unknown tool {tool}"))?;
    if !cfg!(target_os = "macos") {
        return Err("Tool launch is macOS only for now".into());
    }
    let s = spec(t);
    let bundle_dir = app_path(&app, &s).ok_or_else(|| format!("{} is not installed.", s.name))?;
    let bin = bundle_dir.join("Contents/MacOS").join(s.exec);
    let home = app.path().home_dir().map_err(|e| e.to_string())?;

    // Already ours: bring it back into the glass, touch nothing else.
    let ours = our_pids(&app, t);
    let running = running_pids(s.process);
    if running.iter().any(|p| ours.contains(p)) {
        let handle = app.clone();
        thread::spawn(move || {
            if let Err(e) = place_window(&s, &rect) {
                notice(&handle, &s, &e);
            }
        });
        return Ok(());
    }
    if !running.is_empty() {
        quit_and_wait(&s)?;
    }

    // Configure only once the app is not running: these apps rewrite their
    // config on the way out. The key reaches each app the way its format
    // allows without ever being written to a file.
    let mut env: Option<(&str, String)> = None;
    match t {
        Tool::Desktop => {
            let helper = app
                .path()
                .app_config_dir()
                .map_err(|e| e.to_string())?
                .join(HELPER_NAME);
            config::write_helper(&helper)?;
            config::write_claude_desktop(&home, &helper, &models)?;
        }
        Tool::ChatGpt => {
            let key = keychain::get_key().ok_or("No key in the keychain. Connect first.")?;
            chatgpt::write_config(&home, &models)?;
            env = Some(("CONSUS_API_KEY", key));
        }
    }

    let mut cmd = Command::new(&bin);
    if let Some((k, v)) = env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", bin.display()))?;
    let pid = child.id();
    app.state::<Children>().0.lock().unwrap().push((t, pid));
    set_backdrop(&app, true);

    let handle = app.clone();
    thread::spawn(move || {
        if let Err(e) = place_window(&s, &rect) {
            notice(&handle, &s, &e);
        }
        let _ = child.wait();
        let remaining = {
            let children = handle.state::<Children>();
            let mut c = children.0.lock().unwrap();
            c.retain(|(_, p)| *p != pid);
            c.len()
        };
        if remaining == 0 {
            set_backdrop(&handle, false);
        }
        let _ = handle.emit("tool-exited", s.key);
    });
    Ok(())
}

#[tauri::command]
pub fn remove_tool_configs(app: AppHandle) -> Result<(), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let claude = config::remove_claude_desktop(&home);
    let chatgpt = chatgpt::remove_config(&home);
    claude.and(chatgpt)
}

/// On launcher exit the apps it started are asked to quit, then killed if
/// they ignore that. Apps the user opened themselves are left alone.
pub fn terminate_children(app: &AppHandle) {
    let ours: Vec<(Tool, u32)> = {
        let children = app.state::<Children>();
        let guard = children.0.lock().unwrap();
        guard.iter().copied().filter(|(_, p)| alive(*p)).collect()
    };
    if ours.is_empty() {
        return;
    }
    for t in TOOLS {
        if ours.iter().any(|(x, _)| *x == t) {
            ask_to_quit(spec(t).bundle);
        }
    }
    for _ in 0..15 {
        if !ours.iter().any(|(_, p)| alive(*p)) {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
    for (_, pid) in ours {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}
