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

use crate::{chatgpt, claude_code, config, keychain};

/// Claude Desktop runs the launcher binary under this name to fetch the key.
pub const HELPER_NAME: &str = "claude-key-helper";
/// Claude Code's apiKeyHelper runs it under this name and wants the bare key.
pub const CODE_HELPER_NAME: &str = "claude-code-key-helper";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Desktop,
    ChatGpt,
    Code,
}

const TOOLS: [Tool; 3] = [Tool::Desktop, Tool::ChatGpt, Tool::Code];

fn key(t: Tool) -> &'static str {
    match t {
        Tool::Desktop => "desktop",
        Tool::ChatGpt => "chatgpt",
        Tool::Code => "code",
    }
}

fn name(t: Tool) -> &'static str {
    match t {
        Tool::Desktop => "Claude",
        Tool::ChatGpt => "ChatGPT",
        Tool::Code => "Claude Code",
    }
}

fn tool_from_key(k: &str) -> Option<Tool> {
    TOOLS.into_iter().find(|t| key(*t) == k)
}

/// A GUI app in a bundle. Claude Code is a terminal program instead.
#[derive(Clone, Copy)]
struct AppSpec {
    app_name: &'static str,
    exec: &'static str,
    bundle: &'static str,
    process: &'static str,
}

fn app_spec(t: Tool) -> Option<AppSpec> {
    match t {
        Tool::Desktop => Some(AppSpec {
            app_name: "Claude.app",
            exec: "Claude",
            bundle: "com.anthropic.claudefordesktop",
            process: "Claude",
        }),
        Tool::ChatGpt => Some(AppSpec {
            app_name: "ChatGPT.app",
            exec: "ChatGPT",
            bundle: "com.openai.codex",
            process: "ChatGPT",
        }),
        Tool::Code => None,
    }
}

/// Something this launcher started. GUI apps are tracked by pid; Claude Code
/// by the Terminal window and tty it runs in, so no other claude session on
/// the machine is ever touched.
#[derive(Clone)]
pub struct Running {
    pub tool: Tool,
    pub pid: u32,
    pub terminal: Option<(i64, String)>,
}

pub struct Children(pub Mutex<Vec<Running>>);

#[derive(Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The app bundle, system-wide or per-user, whichever exists.
fn app_path(app: &AppHandle, s: &AppSpec) -> Option<PathBuf> {
    let system = Path::new("/Applications").join(s.app_name);
    if system.exists() {
        return Some(system);
    }
    let user = app.path().home_dir().ok()?.join("Applications").join(s.app_name);
    user.exists().then_some(user)
}

/// `claude` as the user's terminal would find it. A GUI app's PATH is
/// minimal, so look in the usual places, then ask the login shell.
fn claude_cli(app: &AppHandle) -> Option<PathBuf> {
    let home = app.path().home_dir().ok()?;
    for p in [home.join(".local/bin/claude"), "/opt/homebrew/bin/claude".into(), "/usr/local/bin/claude".into()] {
        if p.exists() {
            return Some(p);
        }
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let out = Command::new(shell).args(["-ilc", "command -v claude"]).output().ok()?;
    let found = String::from_utf8_lossy(&out.stdout).lines().last()?.trim().to_string();
    let p = PathBuf::from(found);
    p.exists().then_some(p)
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
fn app_icon(app: &AppHandle, t: Tool, bundle_dir: &Path) -> Option<String> {
    let k = key(t);
    if let Some((icns, sig, url)) = ICONS.lock().unwrap().get(k) {
        if icns.starts_with(bundle_dir) && signature(icns).as_ref() == Some(sig) {
            return Some(url.clone());
        }
    }
    let icns = icns_path(bundle_dir)?;
    let sig = signature(&icns)?;
    let secs = sig.0.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let dir = app.path().app_cache_dir().ok()?.join("icons");
    let png = dir.join(format!("{k}-{secs}-{}.png", sig.1));
    if !png.exists() {
        fs::create_dir_all(&dir).ok()?;
        // Convert to a temp name and rename, so an interrupted run never
        // leaves a half-written file that would be served forever.
        let tmp = dir.join(format!("{k}-{secs}-{}.tmp.png", sig.1));
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
    ICONS.lock().unwrap().insert(k, (icns, sig, url.clone()));
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
            let status = if !cfg!(target_os = "macos") {
                ToolStatus { installed: false, icon: None }
            } else if let Some(s) = app_spec(t) {
                let dir = app_path(&app, &s);
                let icon = dir.as_deref().and_then(|d| app_icon(&app, t, d));
                ToolStatus { installed: dir.is_some(), icon }
            } else {
                ToolStatus { installed: claude_cli(&app).is_some(), icon: None }
            };
            (key(t).to_string(), status)
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

/// Whether anything still runs on a terminal tty ("ttys005").
fn tty_busy(tty: &str) -> bool {
    Command::new("ps")
        .args(["-t", tty, "-o", "pid="])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

fn ask_to_quit(bundle: &str) {
    let _ = osascript(&format!("tell application id \"{bundle}\" to quit"));
}

fn quit_and_wait(t: Tool, s: &AppSpec) -> Result<(), String> {
    ask_to_quit(s.bundle);
    for _ in 0..40 {
        if running_pids(s.process).is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(format!("{} is still running. Close it and try again.", name(t)))
}

fn place_window(t: Tool, s: &AppSpec, rect: &Rect) -> Result<(), String> {
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
        return Err(format!("{} did not open a window in time.", name(t)));
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

/// Terminal is scriptable directly, so placing its window needs no
/// Accessibility grant.
fn place_terminal(window: i64, rect: &Rect) -> Result<(), String> {
    let (x1, y1) = (rect.x.round(), rect.y.round());
    let (x2, y2) = ((rect.x + rect.w).round(), (rect.y + rect.h).round());
    osascript(&format!(
        "tell application \"Terminal\"\n\
           set bounds of window id {window} to {{{x1}, {y1}, {x2}, {y2}}}\n\
           activate\n\
         end tell"
    ))
    .map(|_| ())
}

fn close_terminal_window(window: i64) {
    let _ = osascript(&format!("tell application \"Terminal\" to close window id {window}"));
}

fn notice(app: &AppHandle, t: Tool, err: &str) {
    let message = if err.contains("assistive access") {
        format!(
            "{} opened but could not be placed in the launcher. Allow Consus Launcher under System Settings, Privacy and Security, Accessibility.",
            name(t)
        )
    } else {
        format!("{} opened but could not be placed: {err}", name(t))
    };
    let _ = app.emit("tool-notice", json!({ "tool": key(t), "message": message }));
}

/// While a tool is running the launcher is a backdrop: the tool's window
/// stays above it and takes clicks, and the launcher panel still works.
fn set_backdrop(app: &AppHandle, on: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_always_on_bottom(on);
    }
}

fn ours(app: &AppHandle, t: Tool) -> Vec<Running> {
    let children = app.state::<Children>();
    let guard = children.0.lock().unwrap();
    guard.iter().filter(|r| r.tool == t).cloned().collect()
}

fn forget(app: &AppHandle, pid: u32, terminal: Option<&(i64, String)>) -> usize {
    let children = app.state::<Children>();
    let mut c = children.0.lock().unwrap();
    c.retain(|r| !(r.pid == pid && r.terminal.as_ref() == terminal));
    c.len()
}

fn launch_app(app: AppHandle, t: Tool, s: AppSpec, rect: Rect, models: &Value) -> Result<(), String> {
    let bundle_dir = app_path(&app, &s).ok_or_else(|| format!("{} is not installed.", name(t)))?;
    let bin = bundle_dir.join("Contents/MacOS").join(s.exec);
    let home = app.path().home_dir().map_err(|e| e.to_string())?;

    // Already ours: bring it back into the glass, touch nothing else.
    let mine: Vec<u32> = ours(&app, t).iter().map(|r| r.pid).collect();
    let running = running_pids(s.process);
    if running.iter().any(|p| mine.contains(p)) {
        let handle = app.clone();
        thread::spawn(move || {
            if let Err(e) = place_window(t, &s, &rect) {
                notice(&handle, t, &e);
            }
        });
        return Ok(());
    }
    if !running.is_empty() {
        quit_and_wait(t, &s)?;
    }

    // Configure only once the app is not running: these apps rewrite their
    // config on the way out. The key reaches each app the way its format
    // allows without ever being written to a file.
    let mut env: Option<(&str, String)> = None;
    match t {
        Tool::Desktop => {
            let helper = app.path().app_config_dir().map_err(|e| e.to_string())?.join(HELPER_NAME);
            config::write_helper(&helper)?;
            config::write_claude_desktop(&home, &helper, models)?;
        }
        Tool::ChatGpt => {
            let k = keychain::get_key().ok_or("No key in the keychain. Connect first.")?;
            chatgpt::write_config(&home, models)?;
            env = Some(("CONSUS_API_KEY", k));
        }
        Tool::Code => unreachable!("Claude Code is a terminal program"),
    }

    let mut cmd = Command::new(&bin);
    for (k, _) in std::env::vars() {
        if inherited_claude_var(&k) {
            cmd.env_remove(k);
        }
    }
    if let Some((k, v)) = env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", bin.display()))?;
    let pid = child.id();
    app.state::<Children>().0.lock().unwrap().push(Running { tool: t, pid, terminal: None });
    set_backdrop(&app, true);

    let handle = app.clone();
    thread::spawn(move || {
        if let Err(e) = place_window(t, &s, &rect) {
            notice(&handle, t, &e);
        }
        let _ = child.wait();
        if forget(&handle, pid, None) == 0 {
            set_backdrop(&handle, false);
        }
        let _ = handle.emit("tool-exited", key(t));
    });
    Ok(())
}

/// Variables that belong to whatever Claude session the launcher itself was
/// started from, or that could route a tool around the gateway.
fn inherited_claude_var(name: &str) -> bool {
    name.starts_with("CLAUDE") || name.starts_with("ANTHROPIC")
}

fn applescript_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn launch_code(app: AppHandle, rect: Rect, models: &Value) -> Result<(), String> {
    let t = Tool::Code;
    claude_cli(&app).ok_or("Claude Code is not installed.")?;
    let home = app.path().home_dir().map_err(|e| e.to_string())?;

    // Already ours: bring the terminal back into the glass.
    if let Some(r) = ours(&app, t).into_iter().find(|r| r.terminal.as_ref().is_some_and(|(_, tty)| tty_busy(tty))) {
        let (window, _) = r.terminal.clone().unwrap();
        let handle = app.clone();
        thread::spawn(move || {
            if let Err(e) = place_terminal(window, &rect) {
                notice(&handle, t, &e);
            }
        });
        return Ok(());
    }

    let helper = app.path().app_config_dir().map_err(|e| e.to_string())?.join(CODE_HELPER_NAME);
    config::write_helper(&helper)?;
    claude_code::write_settings(&home, &helper, models)?;

    // The user's own ~/.claude is never involved: an isolated profile; every
    // CLAUDE*/ANTHROPIC* variable cleared, which covers both the ones the
    // guide warns can override the gateway and any inherited from a Claude
    // session the launcher was started from; and an empty working folder,
    // because Claude Code treats its starting folder as the project and home
    // would pull in ~/.claude as project settings. The shell does the
    // clearing, since the Terminal window's environment is not ours to set.
    let profile = claude_code::profile_dir(&home);
    let work = claude_code::work_dir(&home);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let command = format!(
        "cd \"{}\"; for v in $(env | grep -oE '^(CLAUDE|ANTHROPIC)[A-Za-z0-9_]*=' | tr -d =); do unset \"$v\"; done; \
         export CLAUDE_CONFIG_DIR=\"{}\"; clear; exec claude",
        work.display(),
        profile.display()
    );
    let cmd = applescript_string(&command);
    let (x1, y1) = (rect.x.round(), rect.y.round());
    let (x2, y2) = ((rect.x + rect.w).round(), (rect.y + rect.h).round());
    // A Terminal that was not running opens its own window on launch; use
    // that one instead of opening a second.
    let out = osascript(&format!(
        "set wasRunning to application \"Terminal\" is running\n\
         tell application \"Terminal\"\n\
           if wasRunning then\n\
             set t to do script \"{cmd}\"\n\
           else\n\
             activate\n\
             repeat 50 times\n\
               if (count of windows) > 0 then exit repeat\n\
               delay 0.1\n\
             end repeat\n\
             if (count of windows) > 0 then\n\
               set t to do script \"{cmd}\" in window 1\n\
             else\n\
               set t to do script \"{cmd}\"\n\
             end if\n\
           end if\n\
           activate\n\
           set bounds of front window to {{{x1}, {y1}, {x2}, {y2}}}\n\
           return (id of front window as text) & \" \" & (tty of t)\n\
         end tell"
    ))?;
    let (window, tty) = out
        .split_once(' ')
        .and_then(|(w, tty)| Some((w.parse::<i64>().ok()?, tty.trim().trim_start_matches("/dev/").to_string())))
        .ok_or_else(|| format!("Terminal gave an unexpected answer: {out}"))?;

    app.state::<Children>()
        .0
        .lock()
        .unwrap()
        .push(Running { tool: t, pid: 0, terminal: Some((window, tty.clone())) });
    set_backdrop(&app, true);

    let handle = app.clone();
    thread::spawn(move || {
        // The shell exec'd claude, so the tty empties when claude exits.
        thread::sleep(Duration::from_secs(2));
        while tty_busy(&tty) {
            thread::sleep(Duration::from_secs(2));
        }
        close_terminal_window(window);
        if forget(&handle, 0, Some(&(window, tty))) == 0 {
            set_backdrop(&handle, false);
        }
        let _ = handle.emit("tool-exited", key(t));
    });
    Ok(())
}

#[tauri::command]
pub async fn launch_tool(app: AppHandle, tool: String, rect: Rect, models: Value) -> Result<(), String> {
    let t = tool_from_key(&tool).ok_or_else(|| format!("unknown tool {tool}"))?;
    if !cfg!(target_os = "macos") {
        return Err("Tool launch is macOS only for now".into());
    }
    match app_spec(t) {
        Some(s) => launch_app(app, t, s, rect, &models),
        None => launch_code(app, rect, &models),
    }
}

#[tauri::command]
pub fn remove_tool_configs(app: AppHandle) -> Result<(), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let claude = config::remove_claude_desktop(&home);
    let chatgpt = chatgpt::remove_config(&home);
    let code = claude_code::remove_settings(&home);
    claude.and(chatgpt).and(code)
}

/// On launcher exit the apps it started are asked to quit, then killed if
/// they ignore that. Apps the user opened themselves are left alone, and
/// so is every claude session the launcher did not start.
pub fn terminate_children(app: &AppHandle) {
    let mine: Vec<Running> = {
        let children = app.state::<Children>();
        let guard = children.0.lock().unwrap();
        guard.iter().cloned().collect()
    };
    for r in &mine {
        if let Some((window, tty)) = &r.terminal {
            let _ = Command::new("pkill").args(["-t", tty]).status();
            close_terminal_window(*window);
        }
    }
    let apps: Vec<&Running> = mine.iter().filter(|r| r.terminal.is_none() && alive(r.pid)).collect();
    if apps.is_empty() {
        return;
    }
    for t in TOOLS {
        if apps.iter().any(|r| r.tool == t) {
            if let Some(s) = app_spec(t) {
                ask_to_quit(s.bundle);
            }
        }
    }
    for _ in 0..15 {
        if !apps.iter().any(|r| alive(r.pid)) {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
    for r in apps {
        let _ = Command::new("kill").arg(r.pid.to_string()).status();
    }
}
