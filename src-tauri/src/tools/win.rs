// Windows: find, set up, and start each tool. Checked on Windows 11 ARM64,
// 2026-09-25.
//
// Store (MSIX) apps: Claude starts through its app execution alias, which
// keeps its package identity. ChatGPT has no alias for the app, so it starts
// from its executable directly, which is also how its environment gets the
// key. Command-line tools open in a console window of their own, with the
// environment set on the process itself, no shell script in between.
// Windows are not placed in the glass yet.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::{
    chatgpt_catalog, claude_code_policy, helper_path, name, ours, output_within, track, Running, Tool,
    CODE_HELPER_NAME, HELPER_NAME, PI_HELPER_NAME, PI_ICON,
};
use crate::models::Target;
use crate::{chatgpt, claude_code, codex, config, keychain, pi};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// A helper command that must not flash a console window over the app.
fn hidden(program: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(program);
    c.creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null());
    c
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

/// Whether a path exists. App execution aliases are reparse points that
/// `exists()` cannot follow, so look at the entry itself.
fn present(p: &Path) -> bool {
    p.symlink_metadata().is_ok()
}

/// Per Store package: its install folder, or None and when that was checked.
static PACKAGES: LazyLock<Mutex<HashMap<&'static str, (Option<PathBuf>, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A Store package's install folder, asked of PowerShell (about a second),
/// remembered; "not installed" is asked again after a minute.
fn package_dir(package: &'static str) -> Option<PathBuf> {
    if let Some((dir, at)) = PACKAGES.lock().unwrap().get(package).cloned() {
        if dir.as_ref().is_some_and(|d| d.exists()) || (dir.is_none() && at.elapsed() < Duration::from_secs(60)) {
            return dir;
        }
    }
    let script = format!("(Get-AppxPackage -Name '{package}' | Select-Object -First 1).InstallLocation");
    let dir = hidden("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.exists());
    PACKAGES.lock().unwrap().insert(package, (dir.clone(), Instant::now()));
    dir
}

fn claude_desktop_exe() -> Option<PathBuf> {
    let local = env_dir("LOCALAPPDATA")?;
    let alias = local.join("Microsoft\\WindowsApps\\claude-desktop.exe");
    if present(&alias) {
        return Some(alias);
    }
    let classic = local.join("AnthropicClaude\\claude.exe");
    classic.exists().then_some(classic)
}

fn chatgpt_exe() -> Option<PathBuf> {
    let exe = package_dir("OpenAI.Codex")?.join("app\\ChatGPT.exe");
    exe.exists().then_some(exe)
}

/// The ChatGPT app's Codex engine: its model catalog comes from it, and it
/// stands in for Codex CLI when that is not installed. Windows refuses to
/// run it from inside the Store package ("Access is denied"; only the app's
/// entry point may run from there), so this is the runnable copy the app
/// keeps in %LOCALAPPDATA%\\OpenAI\\Codex\\bin\\<version>, the newest one.
/// None until ChatGPT has run once.
fn chatgpt_codex() -> Option<PathBuf> {
    let bin = env_dir("LOCALAPPDATA")?.join("OpenAI\\Codex\\bin");
    std::fs::read_dir(bin)
        .ok()?
        .flatten()
        .map(|e| e.path().join("codex.exe"))
        .filter(|p| p.exists())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

/// A command-line tool, from where its installers put it, else the PATH.
fn cli(program: &str) -> Option<PathBuf> {
    let home = env_dir("USERPROFILE");
    let appdata = env_dir("APPDATA");
    let candidates = [
        home.map(|h| h.join(".local\\bin").join(format!("{program}.exe"))),
        appdata.map(|a| a.join("npm").join(format!("{program}.cmd"))),
    ];
    if let Some(p) = candidates.into_iter().flatten().find(|p| p.exists()) {
        return Some(p);
    }
    let out = hidden("where.exe").arg(program).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| PathBuf::from(l.trim()))
        .find(|p| p.exists() && p.extension().is_some_and(|e| e != "ps1"))
}

fn terminal_cli(t: Tool) -> Option<PathBuf> {
    match t {
        Tool::Code => cli("claude"),
        Tool::Codex => cli("codex").or_else(chatgpt_codex),
        Tool::Pi => cli("pi"),
        Tool::Desktop | Tool::ChatGpt => None,
    }
}

/// A file somewhere in a folder tree, found by name (a Store package puts
/// its logos in Assets, assets, or Images).
fn find_file(dir: &Path, file: &str, depth: u32) -> Option<PathBuf> {
    let entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
    if let Some(e) = entries.iter().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(file)) {
        return Some(e.path());
    }
    if depth == 0 {
        return None;
    }
    entries
        .iter()
        .filter(|e| e.path().is_dir() && e.file_name() != "node_modules")
        .find_map(|e| find_file(&e.path(), file, depth - 1))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Per tool: the tile icon, worked out once per run.
static ICONS: LazyLock<Mutex<HashMap<&'static str, Option<String>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The tile icon. The Store packages' own app icons: Claude's full icon;
/// ChatGPT ships its logo only without a background, so it goes on a white
/// rounded square like the Mac icon. Terminal tools wear Windows Terminal's.
fn icon(t: Tool) -> Option<String> {
    let k = super::key(t);
    if let Some(i) = ICONS.lock().unwrap().get(k) {
        return i.clone();
    }
    let png = |package: &'static str, file: &str| {
        let dir = package_dir(package)?;
        std::fs::read(find_file(&dir, file, 3)?).ok()
    };
    let made = match t {
        Tool::Desktop => png("Claude", "Square44x44Logo.targetsize-256.png").map(|b| format!("data:image/png;base64,{}", b64(&b))),
        Tool::ChatGpt => png("OpenAI.Codex", "Square44x44Logo.targetsize-256_altform-lightunplated.png").map(|b| {
            let svg = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 256 256'><rect width='256' height='256' rx='58' fill='#fff'/><image href='data:image/png;base64,{}' x='44' y='44' width='168' height='168'/></svg>",
                b64(&b)
            );
            format!("data:image/svg+xml;base64,{}", b64(svg.as_bytes()))
        }),
        Tool::Code | Tool::Codex => png("Microsoft.WindowsTerminal", "Square44x44Logo.targetsize-256.png")
            .map(|b| format!("data:image/png;base64,{}", b64(&b))),
        Tool::Pi => Some(format!("data:image/svg+xml;base64,{}", b64(PI_ICON.as_bytes()))),
    };
    ICONS.lock().unwrap().insert(k, made.clone());
    made
}

/// Whether Claude Desktop's gateway settings come from a machine-wide
/// registry policy (Intune, Group Policy), which the app obeys over any
/// local config.
pub fn claude_desktop_policy() -> bool {
    ["HKLM\\SOFTWARE\\Policies\\Claude", "HKCU\\SOFTWARE\\Policies\\Claude"].iter().any(|key| {
        hidden("reg").args(["query", key, "/v", "inferenceProvider"]).output().is_ok_and(|o| o.status.success())
    })
}

/// Installed, and the tile's icon.
pub fn status(t: Tool) -> (bool, Option<String>) {
    let installed = match t {
        Tool::Desktop => claude_desktop_exe().is_some(),
        Tool::ChatGpt => chatgpt_exe().is_some(),
        Tool::Code | Tool::Codex | Tool::Pi => terminal_cli(t).is_some(),
    };
    (installed, icon(t))
}

/// Process ids running from inside a folder. By folder, not by name:
/// Windows matches names without case, and Claude Code's claude.exe would
/// otherwise count as Claude Desktop's Claude.exe.
fn pids_in(dir: &Path) -> Vec<u32> {
    let script = format!(
        "Get-CimInstance Win32_Process | Where-Object {{ $_.ExecutablePath -and $_.ExecutablePath.StartsWith('{}', [StringComparison]::OrdinalIgnoreCase) }} | ForEach-Object {{ $_.ProcessId }}",
        dir.display().to_string().replace('\'', "''")
    );
    let Ok(out) = hidden("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect()
}

/// Every running process id, from one tasklist call.
fn all_pids() -> Vec<u32> {
    let Ok(out) = hidden("tasklist").args(["/FO", "CSV", "/NH"]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split("\",\"").nth(1).and_then(|p| p.trim_matches('"').parse().ok()))
        .collect()
}

fn alive(pid: u32) -> bool {
    hidden("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
        .unwrap_or(false)
}

/// Closes a running app so it rereads its config: these apps read it only
/// at startup. Asked first, the way its close button would; but closing the
/// window only hides these apps to the tray, so after a moment it is ended.
/// It is reopened right away, and its conversations live on the server.
fn quit_running(t: Tool, dir: &Path) -> Result<(), String> {
    let running = pids_in(dir);
    if running.is_empty() {
        return Ok(());
    }
    let kill = |force: bool| {
        for pid in &running {
            let mut c = hidden("taskkill");
            c.args(["/PID", &pid.to_string(), "/T"]);
            if force {
                c.arg("/F");
            }
            let _ = c.output();
        }
    };
    kill(false);
    for round in 0..20 {
        thread::sleep(Duration::from_millis(250));
        let now = all_pids();
        if !running.iter().any(|p| now.contains(p)) {
            return Ok(());
        }
        if round == 8 {
            kill(true);
        }
    }
    Err(format!("{} is still running. Close it and try again.", name(t)))
}

/// Whether Claude Desktop or ChatGPT is running.
pub fn running(t: Tool) -> bool {
    app_dir(t).is_some_and(|d| !pids_in(&d).is_empty())
}

/// The folder an app runs from: its Store package, or a classic install.
fn app_dir(t: Tool) -> Option<PathBuf> {
    match t {
        Tool::Desktop => package_dir("Claude").or_else(|| env_dir("LOCALAPPDATA").map(|l| l.join("AnthropicClaude"))),
        Tool::ChatGpt => package_dir("OpenAI.Codex"),
        _ => None,
    }
}

/// Removes the variables a tool must not inherit: another session's, or
/// credentials that would let it reach something other than Consus.
fn scrub(cmd: &mut Command, drop: impl Fn(&str) -> bool) {
    for (k, _) in std::env::vars_os() {
        if drop(&k.to_string_lossy().to_uppercase()) {
            cmd.env_remove(k);
        }
    }
}

fn claude_var(n: &str) -> bool {
    n.starts_with("CLAUDE") || n.starts_with("ANTHROPIC")
}

fn codex_var(n: &str) -> bool {
    n.starts_with("OPENAI") || n.starts_with("CODEX")
}

/// Pi would otherwise offer other providers' models next to Consus.
fn pi_var(n: &str) -> bool {
    ["CLAUDE", "ANTHROPIC", "AWS", "GOOGLE", "GCLOUD", "AZURE"].iter().any(|p| n.starts_with(p))
        || n.ends_with("_API_KEY")
        || n == "HF_TOKEN"
        || n == "COPILOT_GITHUB_TOKEN"
}

/// Runs a command-line tool; npm installs it as a .cmd, which needs cmd.
fn cli_command(exe: &Path) -> Command {
    let script = exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if script {
        let mut c = Command::new("cmd.exe");
        c.args(["/d", "/c"]).arg(exe);
        c
    } else {
        Command::new(exe)
    }
}

pub fn launch(app: AppHandle, t: Tool, models: &Value, target: &Target) -> Result<(), String> {
    let home = env_dir("USERPROFILE").ok_or("Could not find your user folder.")?;
    // Already ours: its window stays where it is (no placement on Windows yet).
    if ours(&app, t).iter().any(|r| alive(r.pid)) {
        return Ok(());
    }
    let helper_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let work = claude_code::work_dir(&home);
    let console = matches!(t, Tool::Code | Tool::Codex | Tool::Pi);
    let mut cmd = match t {
        Tool::Desktop => {
            let exe = claude_desktop_exe().ok_or("Claude is not installed.")?;
            if claude_desktop_policy() {
                // Set up by the organization's policy: open it as it is. If
                // it is already open, starting it again brings it forward.
                if app_dir(t).is_some_and(|d| !pids_in(&d).is_empty()) {
                    let _ = Command::new(exe).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
                    return Ok(());
                }
            } else {
                if let Some(dir) = app_dir(t) {
                    quit_running(t, &dir)?;
                }
                let helper = helper_path(&helper_dir, HELPER_NAME);
                config::write_helper(&helper)?;
                config::write_claude_desktop(&home, &helper, models, target)?;
            }
            let mut c = Command::new(exe);
            scrub(&mut c, claude_var);
            c
        }
        Tool::ChatGpt => {
            let exe = chatgpt_exe().ok_or("ChatGPT is not installed.")?;
            if let Some(dir) = app_dir(t) {
                quit_running(t, &dir)?;
            }
            let key = keychain::get_key().ok_or("No key in the keychain. Connect first.")?;
            let catalog = chatgpt_codex().and_then(|engine| chatgpt_catalog(&app, &engine, models, target));
            chatgpt::write_config(&home, models, target, catalog.as_deref())?;
            let mut c = Command::new(exe);
            scrub(&mut c, claude_var);
            // The template's shell_environment_policy keeps CONSUS_* out of
            // every command the agent runs.
            c.env("CONSUS_API_KEY", key);
            c
        }
        Tool::Code => {
            let exe = terminal_cli(t).ok_or("Claude Code is not installed.")?;
            // Under the organization's policy the launcher writes nothing.
            if !claude_code_policy() {
                let helper = helper_path(&helper_dir, CODE_HELPER_NAME);
                config::write_helper(&helper)?;
                claude_code::write_settings(&home, &helper, models, target)?;
            }
            let mut c = cli_command(&exe);
            scrub(&mut c, claude_var);
            c.env("CLAUDE_CONFIG_DIR", claude_code::profile_dir(&home));
            c
        }
        Tool::Codex => {
            let exe = terminal_cli(t).ok_or("Codex is not installed.")?;
            let key = keychain::get_key().ok_or("No key in the keychain. Connect first.")?;
            let profile = codex::profile_dir(&home);
            std::fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
            let mut list = cli_command(&exe);
            list.args(["debug", "models", "--bundled"]).env("CODEX_HOME", &profile).creation_flags(CREATE_NO_WINDOW);
            let bundled = output_within(&mut list, Duration::from_secs(20)).and_then(|s| serde_json::from_str::<Value>(&s).ok());
            codex::write_config(&home, models, bundled.as_ref(), target)?;
            let mut c = cli_command(&exe);
            scrub(&mut c, codex_var);
            c.env("CODEX_HOME", &profile).env("CONSUS_API_KEY", key);
            c
        }
        Tool::Pi => {
            let exe = terminal_cli(t).ok_or("Pi is not installed.")?;
            let helper = helper_path(&helper_dir, PI_HELPER_NAME);
            config::write_helper(&helper)?;
            pi::write_config(&home, &helper, models, target)?;
            let mut c = cli_command(&exe);
            scrub(&mut c, pi_var);
            c.env("PI_CODING_AGENT_DIR", pi::profile_dir(&home))
                .env("PI_TELEMETRY", "0")
                .env("PI_SKIP_VERSION_CHECK", "1");
            c
        }
    };
    if console {
        std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
        cmd.current_dir(&work).creation_flags(CREATE_NEW_CONSOLE);
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", name(t)))?;
    track(&app, Running { tool: t, pid: child.id(), terminal: None }, move || {
        let _ = child.wait();
    });
    Ok(())
}

/// On launcher exit: apps are asked to close, then ended; a console tool
/// and everything it started is ended. Only processes this launcher started.
pub fn terminate(mine: &[Running]) {
    for r in mine {
        if !alive(r.pid) {
            continue;
        }
        let gui = matches!(r.tool, Tool::Desktop | Tool::ChatGpt);
        if gui {
            let _ = hidden("taskkill").args(["/PID", &r.pid.to_string(), "/T"]).output();
            for _ in 0..15 {
                if !alive(r.pid) {
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
        }
        if alive(r.pid) {
            let _ = hidden("taskkill").args(["/PID", &r.pid.to_string(), "/T", "/F"]).output();
        }
    }
}
