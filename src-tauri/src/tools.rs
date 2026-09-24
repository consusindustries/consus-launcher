// Detect, launch, and place tools. macOS only for now; other targets compile
// and report nothing installed.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

use crate::{chatgpt, claude_code, config, keychain, pi};

/// Claude Desktop runs the launcher binary under this name to fetch the key.
pub const HELPER_NAME: &str = "claude-key-helper";
/// Claude Code's apiKeyHelper runs it under this name and wants the bare key.
pub const CODE_HELPER_NAME: &str = "claude-code-key-helper";
/// Pi's models.json runs it under this name, also for the bare key.
pub const PI_HELPER_NAME: &str = "pi-key-helper";
const TERMINAL_APP: &str = "/System/Applications/Utilities/Terminal.app";
/// Pi's mark from pi.dev on its site's background color.
const PI_ICON: &str = include_str!("../assets/pi.svg");

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Desktop,
    ChatGpt,
    Code,
    Pi,
}

const TOOLS: [Tool; 4] = [Tool::Desktop, Tool::ChatGpt, Tool::Code, Tool::Pi];

fn key(t: Tool) -> &'static str {
    match t {
        Tool::Desktop => "desktop",
        Tool::ChatGpt => "chatgpt",
        Tool::Code => "code",
        Tool::Pi => "pi",
    }
}

fn name(t: Tool) -> &'static str {
    match t {
        Tool::Desktop => "Claude",
        Tool::ChatGpt => "ChatGPT",
        Tool::Code => "Claude Code",
        Tool::Pi => "Pi",
    }
}

fn tool_from_key(k: &str) -> Option<Tool> {
    TOOLS.into_iter().find(|t| key(*t) == k)
}

/// A GUI app in a bundle. A tool without one runs in Terminal.
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
        Tool::Code | Tool::Pi => None,
    }
}

/// Something this launcher started. GUI apps are tracked by pid; terminal
/// tools by the Terminal window and tty they run in, so no other session of
/// the same program on the machine is ever touched.
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

/// Per program: the last answer and when the login shell gave it.
type Cache<T> = LazyLock<Mutex<HashMap<&'static str, T>>>;
static CLIS: Cache<(Option<PathBuf>, Instant)> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// How long "not installed" from the login shell stands before asking again.
const SHELL_RECHECK: Duration = Duration::from_secs(60);

/// A command-line tool as the user's terminal would find it. A GUI app's
/// PATH is minimal, so look in the usual places, then ask the login shell.
/// Answers are remembered, a miss for a minute, so focus-driven refreshes
/// cost a few stats, not a shell (about a second with nvm in the rc file).
fn cli(app: &AppHandle, program: &'static str) -> Option<PathBuf> {
    let cached = CLIS.lock().unwrap().get(program).cloned();
    if let Some((Some(p), _)) = &cached {
        if p.exists() {
            return Some(p.clone());
        }
    }
    let home = app.path().home_dir().ok()?;
    let fixed = [home.join(".local/bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into()];
    let found = fixed.into_iter().map(|d| d.join(program)).find(|p| p.exists());
    if found.is_none() && matches!(cached, Some((None, at)) if at.elapsed() < SHELL_RECHECK) {
        return None;
    }
    let found = found.or_else(|| shell_lookup(program));
    CLIS.lock().unwrap().insert(program, (found.clone(), Instant::now()));
    found
}

/// Asks the user's login shell where a program is. Capped at three seconds
/// and given no stdin, so an rc file that waits for input cannot hang the app.
fn shell_lookup(program: &str) -> Option<PathBuf> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut child = Command::new(shell)
        .args(["-ilc", &format!("command -v {program}")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut done = false;
    for _ in 0..30 {
        if child.try_wait().ok()?.is_some() {
            done = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    if !done {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let p = PathBuf::from(out.lines().last()?.trim());
    (p.is_absolute() && p.exists()).then_some(p)
}

type Signature = (SystemTime, u64);

/// Per tool: the .icns the icon came from, that file's signature, and the
/// data URL, so repeat calls cost one stat instead of process spawns.
static ICONS: Cache<(PathBuf, Signature, String)> = LazyLock::new(|| Mutex::new(HashMap::new()));

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
                let installed = terminal_process(t).is_some_and(|p| cli(&app, p).is_some());
                // Pi has a logo of its own; Claude Code opens in Terminal,
                // so it wears Terminal's icon.
                let icon = match t {
                    Tool::Pi => Some(format!(
                        "data:image/svg+xml;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(PI_ICON)
                    )),
                    _ => app_icon(&app, t, Path::new(TERMINAL_APP)),
                };
                ToolStatus { installed, icon }
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

/// The pids of `program` on a tty. From ps, not pgrep: macOS pgrep -t
/// matches nothing for ttys0NN names. ps may print a full path.
fn tty_pids(tty: &str, program: &str) -> Vec<String> {
    let Ok(o) = Command::new("ps").args(["-t", tty, "-o", "pid=,comm="]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.trim().split_once(' '))
        .filter(|(_, comm)| Path::new(comm.trim()).file_name().is_some_and(|n| n == program))
        .map(|(pid, _)| pid.to_string())
        .collect()
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

/// Closes the launcher's Terminal window, but only while it still has just
/// the one tab: a tab the user added is theirs.
fn close_terminal_window(window: i64) {
    let _ = osascript(&format!(
        "tell application \"Terminal\"\n\
           if (count of tabs of window id {window}) is 1 then close window id {window}\n\
         end tell"
    ));
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

/// Records a started tool and watches it: once `wait` returns, the entry is
/// dropped, the backdrop lifts if nothing else is running, and the UI hears.
fn track(app: &AppHandle, entry: Running, wait: impl FnOnce() + Send + 'static) {
    app.state::<Children>().0.lock().unwrap().push(entry.clone());
    set_backdrop(app, true);
    let handle = app.clone();
    thread::spawn(move || {
        wait();
        let remaining = {
            let children = handle.state::<Children>();
            let mut c = children.0.lock().unwrap();
            c.retain(|r| !(r.pid == entry.pid && r.terminal == entry.terminal));
            c.len()
        };
        if remaining == 0 {
            set_backdrop(&handle, false);
        }
        let _ = handle.emit("tool-exited", key(entry.tool));
    });
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
        Tool::Code | Tool::Pi => return Err(format!("{} is not a desktop app.", name(t))),
    }

    // The app's own output is not the launcher's to read or keep.
    let mut cmd = Command::new(&bin);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    for (k, _) in std::env::vars_os() {
        if inherited_claude_var(&k.to_string_lossy()) {
            cmd.env_remove(k);
        }
    }
    if let Some((k, v)) = env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", bin.display()))?;
    let handle = app.clone();
    track(&app, Running { tool: t, pid: child.id(), terminal: None }, move || {
        if let Err(e) = place_window(t, &s, &rect) {
            notice(&handle, t, &e);
        }
        let _ = child.wait();
    });
    Ok(())
}

/// Variables that belong to whatever Claude session the launcher itself was
/// started from, or that could route a tool around the gateway.
fn inherited_claude_var(name: &str) -> bool {
    name.starts_with("CLAUDE") || name.starts_with("ANTHROPIC")
}

/// Unsets, in shell, every variable whose name matches the pattern, for a
/// Terminal window, whose environment the launcher cannot set directly.
fn scrub(names: &str) -> String {
    format!("for v in $(env | grep -oE \"^({names})=\" | tr -d =); do unset \"$v\"; done;")
}

/// The same scrub as inherited_claude_var, for Claude Code.
const CLAUDE_VARS: &str = "(CLAUDE|ANTHROPIC)[A-Za-z0-9_]*";

/// Every credential Pi would otherwise use to offer another provider's
/// models next to Consus (its env-api-keys list: *_API_KEY, the cloud SDK
/// variables, HF_TOKEN, COPILOT_GITHUB_TOKEN), and the user's own
/// CONSUS_API_KEY, which Pi does not need and the agent's shell should not see.
const PI_VARS: &str = "(CLAUDE|ANTHROPIC|AWS|GOOGLE|GCLOUD|AZURE)[A-Za-z0-9_]*|[A-Za-z0-9_]*_API_KEY|HF_TOKEN|COPILOT_GITHUB_TOKEN";

fn applescript_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The program a terminal tool runs, as PATH and `ps` name it (Pi, a Node
/// script, sets its process title to "pi"). Exhaustive on purpose: a new
/// tool must say whether it is a terminal tool.
fn terminal_process(t: Tool) -> Option<&'static str> {
    match t {
        Tool::Code => Some("claude"),
        Tool::Pi => Some("pi"),
        Tool::Desktop | Tool::ChatGpt => None,
    }
}

/// Configures a terminal tool and returns the POSIX script its Terminal
/// window runs. The per-tool part; everything around it is shared.
fn terminal_script(app: &AppHandle, t: Tool, models: &Value) -> Result<String, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let helper_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    // Both start in the same empty folder, so nothing of the user's is the
    // project: Claude Code would read ~/.claude as project settings from home.
    let work = claude_code::work_dir(&home);
    match t {
        Tool::Code => {
            let cli = cli(app, "claude").ok_or("Claude Code is not installed.")?;
            let helper = helper_dir.join(CODE_HELPER_NAME);
            config::write_helper(&helper)?;
            claude_code::write_settings(&home, &helper, models)?;
            fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
            // The user's own ~/.claude is never involved: an isolated profile
            // and a scrubbed environment.
            Ok(format!(
                "cd \"{}\" || exit 1; {} export CLAUDE_CONFIG_DIR=\"{}\"; clear; exec \"{}\"",
                work.display(),
                scrub(CLAUDE_VARS),
                claude_code::profile_dir(&home).display(),
                cli.display()
            ))
        }
        Tool::Pi => {
            let cli = cli(app, "pi").ok_or("Pi is not installed.")?;
            let helper = helper_dir.join(PI_HELPER_NAME);
            config::write_helper(&helper)?;
            pi::write_config(&home, &helper, models)?;
            fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
            // pi is a Node script; the node it was installed with (nvm, the
            // installer's own) sits next to it. The two PI_ switches turn off
            // Pi's calls to pi.dev (install telemetry, update check). Not
            // PI_OFFLINE: that also stops Pi fetching fd and ripgrep, and its
            // find and grep tools fail without them.
            let bin = cli.parent().ok_or("Pi is not installed.")?;
            Ok(format!(
                "cd \"{}\" || exit 1; {} export PATH=\"{}:$PATH\" PI_CODING_AGENT_DIR=\"{}\" PI_TELEMETRY=0 PI_SKIP_VERSION_CHECK=1; clear; exec \"{}\"",
                work.display(),
                scrub(PI_VARS),
                bin.display(),
                pi::profile_dir(&home).display(),
                cli.display()
            ))
        }
        Tool::Desktop | Tool::ChatGpt => Err(format!("{} is not a terminal tool.", name(t))),
    }
}

/// Opens a terminal tool in a Terminal window placed in the glass. The
/// script always runs under /bin/sh whatever the user's login shell is
/// (fish and nushell do not parse POSIX), single-quoted, so it must not
/// contain a single quote; macOS account names cannot.
fn launch_terminal(app: AppHandle, t: Tool, rect: Rect, models: &Value) -> Result<(), String> {
    // Already ours: bring the terminal back into the glass.
    if let Some((window, _)) = ours(&app, t).into_iter().filter_map(|r| r.terminal).find(|(_, tty)| tty_busy(tty)) {
        let handle = app.clone();
        thread::spawn(move || {
            if let Err(e) = place_terminal(window, &rect) {
                notice(&handle, t, &e);
            }
        });
        return Ok(());
    }

    let inner = terminal_script(&app, t, models)?;
    if inner.contains('\'') {
        return Err("A path contains a quote character, which the launcher cannot pass to Terminal.".into());
    }
    let cmd = applescript_string(&format!("exec /bin/sh -c '{inner}'"));
    // Always a window of our own, found by its tab's tty, never by which
    // window is in front. A Terminal that a tell block starts also opens
    // its default window, so a Terminal that is not running is started with
    // `launch` first, which opens none (tested cold, 2026-09-24).
    let out = osascript(&format!(
        "if application \"Terminal\" is not running then launch application \"Terminal\"\n\
         tell application \"Terminal\"\n\
           set t to do script \"{cmd}\"\n\
           set tt to tty of t\n\
           repeat with w in windows\n\
             repeat with x in tabs of w\n\
               if tty of x is tt then return (id of w as text) & \" \" & tt\n\
             end repeat\n\
           end repeat\n\
           return \"0 \" & tt\n\
         end tell"
    ))?;
    let mut parts = out.split_whitespace();
    let (window, tty) = (|| {
        let w = parts.next()?.parse::<i64>().ok().filter(|w| *w != 0)?;
        Some((w, parts.next()?.trim_start_matches("/dev/").to_string()))
    })()
    .ok_or_else(|| format!("Terminal gave an unexpected answer: {out}"))?;
    if let Err(e) = place_terminal(window, &rect) {
        notice(&app, t, &e);
    }

    let watched = tty.clone();
    track(&app, Running { tool: t, pid: 0, terminal: Some((window, tty)) }, move || {
        // The script exec'd the tool, so the tty empties when it exits.
        thread::sleep(Duration::from_secs(2));
        while tty_busy(&watched) {
            thread::sleep(Duration::from_secs(2));
        }
        close_terminal_window(window);
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
        None => launch_terminal(app, t, rect, &models),
    }
}

#[tauri::command]
pub fn remove_tool_configs(app: AppHandle) -> Result<(), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let claude = config::remove_claude_desktop(&home);
    let chatgpt = chatgpt::remove_config(&home);
    let code = claude_code::remove_settings(&home);
    let pi = pi::remove_config(&home);
    claude.and(chatgpt).and(code).and(pi)
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
    // Only the tool's own program on the tracked tty: if that tty was freed
    // and reused by the time the launcher quits, whatever else is on it is
    // not ours.
    for r in &mine {
        if let Some((window, tty)) = &r.terminal {
            if let Some(program) = terminal_process(r.tool) {
                for pid in tty_pids(tty, program) {
                    let _ = Command::new("kill").arg(pid).status();
                }
            }
            for _ in 0..10 {
                if !tty_busy(tty) {
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
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
