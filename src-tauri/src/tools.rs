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

use crate::models::Target;

#[cfg(windows)]
mod win;
use crate::{chatgpt, claude_code, codex, config, keychain, pi, settings};

/// Claude Desktop runs the launcher binary under this name to fetch the key.
pub const HELPER_NAME: &str = "claude-key-helper";
/// Claude Code's apiKeyHelper runs it under this name and wants the bare key.
pub const CODE_HELPER_NAME: &str = "claude-code-key-helper";
/// Pi's models.json runs it under this name, also for the bare key.
pub const PI_HELPER_NAME: &str = "pi-key-helper";
/// Codex's Terminal script runs it under this name, also for the bare key.
pub const CODEX_HELPER_NAME: &str = "codex-key-helper";
const TERMINAL_APP: &str = "/System/Applications/Utilities/Terminal.app";

/// Where a key helper lives: a link to this binary, named for its caller.
/// On Windows it needs the .exe to be runnable.
fn helper_path(dir: &Path, helper: &str) -> PathBuf {
    if cfg!(windows) {
        dir.join(format!("{helper}.exe"))
    } else {
        dir.join(helper)
    }
}
/// Pi's mark from pi.dev on its site's background color.
const PI_ICON: &str = include_str!("../assets/pi.svg");

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Desktop,
    ChatGpt,
    Code,
    Codex,
    Pi,
}

const TOOLS: [Tool; 5] = [Tool::Desktop, Tool::ChatGpt, Tool::Code, Tool::Codex, Tool::Pi];

fn key(t: Tool) -> &'static str {
    match t {
        Tool::Desktop => "desktop",
        Tool::ChatGpt => "chatgpt",
        Tool::Code => "code",
        Tool::Codex => "codex",
        Tool::Pi => "pi",
    }
}

fn name(t: Tool) -> &'static str {
    match t {
        Tool::Desktop => "Claude",
        Tool::ChatGpt => "ChatGPT",
        Tool::Code => "Claude Code",
        Tool::Codex => "Codex",
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
        Tool::Code | Tool::Codex | Tool::Pi => None,
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

/// A command's stdout if it succeeds within `limit`. No stdin, so nothing
/// can wait on input; stdout is read as it comes, so a large output cannot
/// fill the pipe and stall the command. The read is bounded too: something
/// the command left running in the background can hold the pipe open.
fn output_within(cmd: &mut Command, limit: Duration) -> Option<String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut pipe = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut out = String::new();
        let _ = tx.send(pipe.read_to_string(&mut out).ok().map(|_| out));
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let out = rx.recv_timeout(limit.saturating_sub(start.elapsed())).ok()??;
    status.success().then_some(out)
}

/// The ChatGPT app's model catalog, built like Codex CLI's from the Codex
/// engine inside the app, into the launcher's own folder. Without it the app
/// shows Consus models as "Custom" and cannot list them. None when the
/// engine cannot list its models.
fn chatgpt_catalog(app: &AppHandle, engine: &Path, models: &Value, target: &Target) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    // An empty home of its own, so the listing never reads the user's ~/.codex.
    let probe = dir.join("codex-probe");
    fs::create_dir_all(&probe).ok()?;
    let mut cmd = Command::new(engine);
    cmd.args(["debug", "models", "--bundled"]).env("CODEX_HOME", &probe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let bundled: Value = serde_json::from_str(&output_within(&mut cmd, Duration::from_secs(20))?).ok()?;
    let list = codex::catalog(&bundled, models, target);
    if list.is_empty() {
        return None;
    }
    let path = dir.join(CHATGPT_CATALOG);
    let text = serde_json::to_string_pretty(&json!({ "models": list })).ok()?;
    fs::write(&path, text + "\n").ok()?;
    Some(path)
}

const CHATGPT_CATALOG: &str = "chatgpt-models.json";

/// Asks the user's login shell where a program is, for at most three
/// seconds, so an rc file that hangs cannot hang the app.
fn shell_lookup(program: &str) -> Option<PathBuf> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let out = output_within(
        Command::new(shell).args(["-ilc", &format!("command -v {program}")]),
        Duration::from_secs(3),
    )?;
    let p = PathBuf::from(out.lines().last()?.trim());
    (p.is_absolute() && p.exists()).then_some(p)
}

/// The Codex CLI: the user's own, else the one inside the ChatGPT app.
fn codex_cli(app: &AppHandle) -> Option<PathBuf> {
    cli(app, "codex").or_else(|| {
        let bundled = app_path(app, &app_spec(Tool::ChatGpt)?)?.join("Contents/Resources/codex");
        bundled.exists().then_some(bundled)
    })
}

/// The program a terminal tool runs, found the way the user's terminal
/// would find it.
fn terminal_cli(app: &AppHandle, t: Tool) -> Option<PathBuf> {
    match t {
        Tool::Codex => codex_cli(app),
        _ => cli(app, terminal_process(t)?),
    }
}

type Signature = (SystemTime, u64);

/// Per tool: the image the icon came from, that file's signature, and the
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

/// The app's own icon as a PNG data URL, from the bundle's .icns.
fn app_icon(app: &AppHandle, t: Tool, bundle_dir: &Path) -> Option<String> {
    cached_icon(t, bundle_dir).or_else(|| image_icon(app, t, &icns_path(bundle_dir)?))
}

/// The remembered icon, if it came from under `within` and that file has
/// not changed since.
fn cached_icon(t: Tool, within: &Path) -> Option<String> {
    let icons = ICONS.lock().unwrap();
    let (src, sig, url) = icons.get(key(t))?;
    (src.starts_with(within) && signature(src).as_ref() == Some(sig)).then(|| url.clone())
}

/// An image file (.icns or .png) as a 128-point PNG data URL, converted
/// once. The cache file is named by the source's mtime and size, so an app
/// update with a new icon is picked up even when the updater preserves old
/// file dates. None on other platforms or if anything is missing.
fn image_icon(app: &AppHandle, t: Tool, src: &Path) -> Option<String> {
    let k = key(t);
    let sig = signature(src)?;
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
            .arg(src)
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
    ICONS.lock().unwrap().insert(k, (src.to_path_buf(), sig, url.clone()));
    Some(url)
}

/// Where device management puts Claude Code's machine-wide settings.
fn claude_code_policy_files() -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![
            PathBuf::from("C:\\Program Files\\ClaudeCode\\managed-settings.json"),
            PathBuf::from("C:\\ProgramData\\ClaudeCode\\managed-settings.json"),
        ]
    } else {
        vec![PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json")]
    }
}

/// Whether a machine-wide Claude Code settings file decides how it connects
/// (a key helper, a base URL, or a key). One that cannot be read counts too:
/// Claude Code itself stops on it, and it is not the launcher's to override.
fn claude_code_policy() -> bool {
    claude_code_policy_files()
        .iter()
        .filter(|p| p.exists())
        .any(|p| claude_code::read_object(p).map_or(true, |doc| sets_connection(&doc)))
}

/// Whether Claude Code settings decide where requests go or which key they carry.
fn sets_connection(doc: &serde_json::Map<String, Value>) -> bool {
    const CONNECT: [&str; 3] = ["ANTHROPIC_BASE_URL", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];
    doc.contains_key("apiKeyHelper")
        || doc.get("env").and_then(Value::as_object).is_some_and(|e| CONNECT.iter().any(|k| e.contains_key(*k)))
}

/// Whether Claude Desktop's gateway settings come from a machine-wide policy
/// (a configuration profile), which the app obeys over any local config.
#[cfg(target_os = "macos")]
fn claude_desktop_policy() -> bool {
    let file = "com.anthropic.claudefordesktop.plist";
    let base = Path::new("/Library/Managed Preferences");
    let mut plists = vec![base.join(file)];
    if let Ok(user) = std::env::var("USER") {
        plists.push(base.join(user).join(file));
    }
    plists.iter().filter(|p| p.exists()).any(|p| {
        Command::new("plutil")
            .args(["-extract", "inferenceProvider", "raw", "-o", "-"])
            .arg(p)
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[cfg(windows)]
fn claude_desktop_policy() -> bool {
    win::claude_desktop_policy()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn claude_desktop_policy() -> bool {
    false
}

/// Whether the organization already sets this tool up through a machine-wide
/// policy. Then the launcher writes nothing for it and just opens it: the
/// admin's policy is the authority, and the tile says so.
fn org_policy(t: Tool) -> bool {
    match t {
        Tool::Code => claude_code_policy(),
        Tool::Desktop => claude_desktop_policy(),
        Tool::ChatGpt | Tool::Codex | Tool::Pi => false,
    }
}

#[derive(Serialize)]
pub struct ToolStatus {
    /// Whether the org's settings show this tool at all.
    pub allowed: bool,
    /// Set up by a machine-wide policy, which the launcher leaves alone.
    pub managed: bool,
    pub installed: bool,
    pub icon: Option<String>,
}

#[tauri::command]
pub async fn detect_tools(app: AppHandle) -> HashMap<String, ToolStatus> {
    // Invalid settings hide every tool; the UI shows why.
    let settings = settings::current().ok();
    TOOLS
        .into_iter()
        .map(|t| {
            let allowed = settings.as_ref().is_some_and(|s| s.allows(key(t)));
            let status = if !allowed {
                ToolStatus { allowed, managed: false, installed: false, icon: None }
            } else if !cfg!(target_os = "macos") {
                let (installed, icon) = other_status(t);
                ToolStatus { allowed, managed: installed && org_policy(t), installed, icon }
            } else if let Some(s) = app_spec(t) {
                let dir = app_path(&app, &s);
                let icon = dir.as_deref().and_then(|d| app_icon(&app, t, d));
                let installed = dir.is_some();
                ToolStatus { allowed, managed: installed && org_policy(t), installed, icon }
            } else {
                let installed = terminal_cli(&app, t).is_some();
                // Pi has a logo of its own and the ChatGPT app carries
                // Codex's; Claude Code, or Codex without that app, opens in
                // Terminal, so it wears Terminal's icon.
                let codex_png = app_spec(Tool::ChatGpt)
                    .and_then(|s| app_path(&app, &s))
                    .map(|d| d.join("Contents/Resources/icon-codex-dark-color.png"))
                    .filter(|p| p.exists());
                let icon = match (t, codex_png) {
                    (Tool::Pi, _) => Some(format!(
                        "data:image/svg+xml;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(PI_ICON)
                    )),
                    (Tool::Codex, Some(png)) => cached_icon(t, &png).or_else(|| image_icon(&app, t, &png)),
                    _ => app_icon(&app, t, Path::new(TERMINAL_APP)),
                };
                ToolStatus { allowed, managed: installed && org_policy(t), installed, icon }
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

fn launch_app(app: AppHandle, t: Tool, s: AppSpec, rect: Rect, models: &Value, target: &Target) -> Result<(), String> {
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
    // Set up by the organization's policy: open it as it is. An instance
    // the user already has open is simply brought forward and placed.
    let policy = org_policy(t);
    if policy && !running.is_empty() {
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
        Tool::Desktop if policy => {}
        Tool::Desktop => {
            let helper = helper_path(&app.path().app_config_dir().map_err(|e| e.to_string())?, HELPER_NAME);
            config::write_helper(&helper)?;
            config::write_claude_desktop(&home, &helper, models, target)?;
        }
        Tool::ChatGpt => {
            let k = keychain::get_key().ok_or("No key in the keychain. Connect first.")?;
            let catalog = chatgpt_catalog(&app, &bundle_dir.join("Contents/Resources/codex"), models, target);
            chatgpt::write_config(&home, models, target, catalog.as_deref())?;
            env = Some(("CONSUS_API_KEY", k));
        }
        Tool::Code | Tool::Codex | Tool::Pi => return Err(format!("{} is not a desktop app.", name(t))),
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
/// Anything that could point Codex at OpenAI directly or at another home.
const CODEX_VARS: &str = "(OPENAI|CODEX)[A-Za-z0-9_]*";

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
        Tool::Codex => Some("codex"),
        Tool::Pi => Some("pi"),
        Tool::Desktop | Tool::ChatGpt => None,
    }
}

/// Configures a terminal tool and returns the POSIX script its Terminal
/// window runs. The per-tool part; everything around it is shared.
fn terminal_script(app: &AppHandle, t: Tool, models: &Value, target: &Target) -> Result<String, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let helper_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    // Both start in the same empty folder, so nothing of the user's is the
    // project: Claude Code would read ~/.claude as project settings from home.
    let work = claude_code::work_dir(&home);
    match t {
        Tool::Code => {
            let cli = cli(app, "claude").ok_or("Claude Code is not installed.")?;
            // Under the organization's policy the launcher writes nothing;
            // the profile folder still keeps the user's own ~/.claude apart.
            if !claude_code_policy() {
                let helper = helper_path(&helper_dir, CODE_HELPER_NAME);
                config::write_helper(&helper)?;
                claude_code::write_settings(&home, &helper, models, target)?;
            }
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
        Tool::Codex => {
            let cli = codex_cli(app).ok_or("Codex is not installed.")?;
            keychain::get_key().ok_or("No key in the keychain. Connect first.")?;
            let helper = helper_path(&helper_dir, CODEX_HELPER_NAME);
            config::write_helper(&helper)?;
            let profile = codex::profile_dir(&home);
            fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
            // An npm install is a Node script; its node sits next to it, and
            // an app started from Finder has no node on its PATH.
            let bin = cli.parent().ok_or("Codex is not installed.")?;
            let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
            // The catalog is cloned from this Codex's own model list.
            let bundled = output_within(
                Command::new(&cli)
                    .args(["debug", "models", "--bundled"])
                    .env("CODEX_HOME", &profile)
                    .env("PATH", path),
                Duration::from_secs(10),
            )
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
            if bundled.is_none() {
                let _ = app.emit(
                    "tool-notice",
                    json!({ "tool": key(t), "message": "Codex could not list its models, so /model will not show the Consus ones this time." }),
                );
            }
            codex::write_config(&home, models, bundled.as_ref(), target)?;
            fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
            // Codex reads the key from CONSUS_API_KEY (env_http_headers), so
            // the script fetches it from the keychain helper into Codex's
            // environment only; the template's shell_environment_policy
            // keeps CONSUS_* out of every command the agent runs. No key, no
            // Codex: it would start and fail every request with no reason shown.
            Ok(format!(
                "cd \"{}\" || exit 1; {} CONSUS_API_KEY=\"$(\"{}\")\" || exit 1; export PATH=\"{}:$PATH\" CONSUS_API_KEY CODEX_HOME=\"{}\"; clear; exec \"{}\"",
                work.display(),
                scrub(CODEX_VARS),
                helper.display(),
                bin.display(),
                profile.display(),
                cli.display()
            ))
        }
        Tool::Pi => {
            let cli = cli(app, "pi").ok_or("Pi is not installed.")?;
            let helper = helper_path(&helper_dir, PI_HELPER_NAME);
            config::write_helper(&helper)?;
            pi::write_config(&home, &helper, models, target)?;
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
fn launch_terminal(app: AppHandle, t: Tool, rect: Rect, models: &Value, target: &Target) -> Result<(), String> {
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

    let inner = terminal_script(&app, t, models, target)?;
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
    let settings = settings::current()?;
    if !settings.allows(key(t)) {
        return Err(format!("Your organization has not enabled {} in the launcher.", name(t)));
    }
    if !cfg!(target_os = "macos") {
        let _ = rect;
        return other_launch(app, t, &models, &settings.target);
    }
    match app_spec(t) {
        Some(s) => launch_app(app, t, s, rect, &models, &settings.target),
        None => launch_terminal(app, t, rect, &models, &settings.target),
    }
}

#[tauri::command]
pub fn remove_tool_configs(app: AppHandle) -> Result<(), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let claude = config::remove_claude_desktop(&home);
    let catalog = app.path().app_config_dir().map(|d| d.join(CHATGPT_CATALOG)).unwrap_or_default();
    let chatgpt = chatgpt::remove_config(&home, &catalog);
    let code = claude_code::remove_settings(&home);
    let codex = codex::remove_config(&home);
    let pi = pi::remove_config(&home);
    let _ = fs::remove_file(&catalog);
    claude.and(chatgpt).and(code).and(codex).and(pi)
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
    if !cfg!(target_os = "macos") {
        other_terminate(&mine);
        return;
    }
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

// Windows goes through tools/win.rs; other platforms have no tools yet.

#[cfg(windows)]
fn other_status(t: Tool) -> (bool, Option<String>) {
    win::status(t)
}

#[cfg(windows)]
fn other_launch(app: AppHandle, t: Tool, models: &Value, target: &Target) -> Result<(), String> {
    win::launch(app, t, models, target)
}

#[cfg(windows)]
fn other_terminate(mine: &[Running]) {
    win::terminate(mine)
}

#[cfg(not(windows))]
fn other_status(_: Tool) -> (bool, Option<String>) {
    (false, None)
}

#[cfg(not(windows))]
fn other_launch(_: AppHandle, _: Tool, _: &Value, _: &Target) -> Result<(), String> {
    Err("Tool launch is not supported on this platform yet.".into())
}

#[cfg(not(windows))]
fn other_terminate(_: &[Running]) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_counts_only_when_it_sets_the_connection() {
        let obj = |v: Value| v.as_object().unwrap().clone();
        assert!(sets_connection(&obj(json!({ "apiKeyHelper": "x" }))));
        assert!(sets_connection(&obj(json!({ "env": { "ANTHROPIC_BASE_URL": "https://api.consus.io" } }))));
        assert!(!sets_connection(&obj(json!({ "permissions": { "deny": ["Bash(rm:*)"] } }))));
        assert!(!sets_connection(&obj(json!({ "env": { "DISABLE_TELEMETRY": "1" } }))));
    }
}
