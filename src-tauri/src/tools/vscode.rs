// VS Code: find it, start the launcher's own instance, and close only that.
//
// The user's own VS Code is the same app, so nothing here finds, counts or
// quits a VS Code by its name, bundle id or install folder. The launcher's
// instance is the process it started with --user-data-dir: VS Code keeps
// one main process per user-data-dir, so that pid is ours even while the
// user's own VS Code runs, and starting the binary again with the same
// folder hands over to it and exits. Its window is not placed in the glass.
//
// A new instance opens on ~/Consus, as every tool starts there; VS Code
// asks the person to trust it, and lists no model until they do (its trust
// prompt is left on). Later starts pass no folder, so VS Code reopens what
// the person had open.
//
// Under the organization's VS Code lock (managed policies) the launcher
// still writes everything here: policies cannot carry the model list, the
// state file or the default model. Only the tile's label changes.
//
// The settings are written by crate::vscode (src/vscode.rs).

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::terminal::VSCODE_SCRUB;
use super::{key, ours, track_with, Running, Tool};
use crate::models::Target;

/// The oldest VS Code the four "Update API Key" steps were checked on
/// (docs/vscode.md, check 11).
const MIN_VERSION: [u32; 3] = [1, 147, 0];

/// Where an installed VS Code keeps its binary and its app folder, which
/// holds package.json (the version) on both systems.
struct Install {
    bin: PathBuf,
    app_dir: PathBuf,
    #[cfg(target_os = "macos")]
    bundle: PathBuf,
}

#[cfg(target_os = "macos")]
fn find(app: &AppHandle) -> Option<Install> {
    let bundle = super::bundle_path(app, "Visual Studio Code.app")?;
    let bin = bundle.join("Contents/MacOS/Code");
    bin.is_file().then(|| Install { bin, app_dir: bundle.join("Contents/Resources/app"), bundle })
}

#[cfg(windows)]
fn find(_: &AppHandle) -> Option<Install> {
    let bin = windows::code_exe()?;
    let app_dir = windows::app_dir(bin.parent()?)?;
    Some(Install { bin, app_dir })
}

#[cfg(not(any(target_os = "macos", windows)))]
fn find(_: &AppHandle) -> Option<Install> {
    None
}

/// "1.147.0" from the install's package.json.
fn version(app_dir: &Path) -> Option<Vec<u32>> {
    let text = std::fs::read_to_string(app_dir.join("package.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    parse_version(v.get("version")?.as_str()?)
}

fn parse_version(s: &str) -> Option<Vec<u32>> {
    // "1.147.0-insider" counts as 1.147.0.
    s.split(['-', '+']).next()?.split('.').map(|p| p.parse().ok()).collect()
}

/// Why this VS Code is too old to use, if it is. A version that cannot be
/// read does not block.
fn too_old(app_dir: &Path) -> Option<String> {
    let v = version(app_dir)?;
    (v.as_slice() < MIN_VERSION.as_slice()).then(|| {
        let min = MIN_VERSION.map(|n| n.to_string()).join(".");
        format!("Update VS Code to {min} or later to use it here.")
    })
}

#[cfg(target_os = "macos")]
fn icon(app: &AppHandle, i: &Install) -> Option<String> {
    super::app_icon(app, Tool::VsCode, &i.bundle)
}

#[cfg(windows)]
fn icon(_: &AppHandle, i: &Install) -> Option<String> {
    windows::icon(&i.app_dir)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn icon(_: &AppHandle, _: &Install) -> Option<String> {
    None
}

/// Installed, the tile's icon, and why the tile is greyed (too old).
pub fn status(app: &AppHandle) -> (bool, Option<String>, Option<String>) {
    match find(app) {
        Some(i) => (true, icon(app, &i), too_old(&i.app_dir)),
        None => (false, None, None),
    }
}

/// The binary with the launcher's two folders, and the folder to open on a
/// new instance; never one of the `code` command's subcommands, which act
/// on the user's own VS Code.
fn command(bin: &Path, home: &Path, open: Option<&Path>) -> Command {
    let mut c = Command::new(bin);
    c.arg("--user-data-dir")
        .arg(crate::vscode::data_dir(home))
        .arg("--extensions-dir")
        .arg(crate::vscode::extensions_dir(home));
    if let Some(folder) = open {
        c.arg(folder);
    }
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, _) in std::env::vars_os() {
        let name = k.to_string_lossy();
        // Windows matches variable names without case.
        let name = if cfg!(windows) { name.to_uppercase() } else { name.into_owned() };
        if VSCODE_SCRUB.matches(&name) {
            c.env_remove(k);
        }
    }
    c
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    super::win::alive(pid)
}

#[cfg(not(windows))]
fn alive(pid: u32) -> bool {
    super::alive(pid)
}

/// The launcher's running instance, if any.
fn running(app: &AppHandle) -> Option<u32> {
    ours(app, Tool::VsCode).into_iter().map(|r| r.pid).find(|p| alive(*p))
}

pub fn launch(app: AppHandle, models: &Value, target: &Target) -> Result<(), String> {
    let install = find(&app).ok_or("VS Code is not installed.")?;
    if let Some(why) = too_old(&install.app_dir) {
        return Err(why);
    }
    let home = app.path().home_dir().map_err(|e| e.to_string())?;

    // Already ours: starting it again with the same folders brings its
    // window forward; that second process hands over and exits. Nothing is
    // rewritten under a running VS Code.
    if running(&app).is_some() {
        let mut again = command(&install.bin, &home, None).spawn().map_err(|e| format!("VS Code: {e}"))?;
        thread::spawn(move || {
            let _ = again.wait();
        });
        return Ok(());
    }

    // The config first: if it fails, no state file is left behind, so the
    // next try still counts as a new instance and opens ~/Consus.
    crate::vscode::write_config(&home, models, target)?;
    let new = crate::vscode::write_state(&home)?;
    let ext = crate::vscode::extensions_dir(&home);
    std::fs::create_dir_all(&ext).map_err(|e| format!("{}: {e}", ext.display()))?;
    let work = crate::claude_code::work_dir(&home);
    if new {
        std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    }

    let mut child = command(&install.bin, &home, new.then_some(work.as_path())).spawn().map_err(|e| format!("VS Code: {e}"))?;
    let pid = child.id();
    track_with(&app, Running { tool: Tool::VsCode, pid, terminal: None }, false, move || {
        let _ = child.wait();
        pid
    });
    Ok(())
}

/// How long VS Code gets to close by itself before it is ended. On Windows
/// it took about 9 seconds (1.147.0, check 4).
const CLOSE_WAIT: Duration = Duration::from_secs(15);

/// Closes the launcher's VS Code, if it runs, at sign out and at launcher
/// exit: asked first, ended if it does not close. Only the pid this
/// launcher started (and on Windows its children).
pub fn quit_ours(app: &AppHandle) {
    for pid in ours(app, Tool::VsCode).into_iter().map(|r| r.pid).filter(|p| alive(*p)) {
        ask_to_close(pid);
        for _ in 0..(CLOSE_WAIT.as_millis() / 200) {
            if !alive(pid) {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        if alive(pid) {
            force_close(pid);
        }
    }
}

/// The main process's windows are asked to close, and VS Code then ends its
/// own helpers. Not /T: Windows refuses a polite close of a tree whose
/// helpers can only be ended by force, so nothing would close (check 4).
#[cfg(windows)]
fn ask_to_close(pid: u32) {
    let _ = super::win::hidden("taskkill").args(["/PID", &pid.to_string()]).output();
}

#[cfg(windows)]
fn force_close(pid: u32) {
    let _ = super::win::hidden("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output();
}

#[cfg(not(windows))]
fn ask_to_close(pid: u32) {
    let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
}

#[cfg(not(windows))]
fn force_close(pid: u32) {
    let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
}

/// "Copy key" in the VS Code steps: the key onto the clipboard, from the
/// keychain, never through the webview. Cleared after two minutes.
#[tauri::command]
pub async fn vscode_copy_key(app: AppHandle) -> Result<(), String> {
    let settings = crate::settings::current()?;
    if !settings.allows(key(Tool::VsCode)) {
        return Err("Your organization has not enabled VS Code in the launcher.".into());
    }
    let k = crate::keychain::get_key().ok_or("No key in the keychain. Sign in first.")?;
    #[cfg(windows)]
    let owner = app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map_or(0, |h| h.0 as isize);
    #[cfg(not(windows))]
    let owner = {
        let _ = &app;
        0
    };
    crate::clipboard::copy_secret(&k, owner)
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use base64::Engine as _;

    use super::super::win::{env_dir, hidden};

    /// The last registry answer and when it was asked; a miss is asked
    /// again after a minute.
    static FOUND: Mutex<Option<(Option<PathBuf>, Instant)>> = Mutex::new(None);

    /// Code.exe: the per-user install, the all-users one, else wherever the
    /// installer's uninstall entry says (a custom folder). Never
    /// bin\code.cmd, the command-line wrapper.
    pub fn code_exe() -> Option<PathBuf> {
        let fixed = [
            env_dir("LOCALAPPDATA").map(|l| l.join("Programs\\Microsoft VS Code\\Code.exe")),
            env_dir("ProgramFiles").map(|p| p.join("Microsoft VS Code\\Code.exe")),
        ];
        if let Some(p) = fixed.into_iter().flatten().find(|p| p.is_file()) {
            return Some(p);
        }
        if let Some((found, at)) = FOUND.lock().unwrap().clone() {
            if found.as_ref().is_some_and(|p| p.is_file()) || (found.is_none() && at.elapsed() < Duration::from_secs(60)) {
                return found;
            }
        }
        let script = "$k = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*','HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*','HKLM:\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*'; \
             Get-ItemProperty $k -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like 'Microsoft Visual Studio Code*' -and $_.DisplayName -notlike '*Insiders*' -and $_.InstallLocation } | ForEach-Object { $_.InstallLocation }";
        let found = hidden("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout).lines().map(|l| PathBuf::from(l.trim()).join("Code.exe")).find(|p| p.is_file())
            });
        *FOUND.lock().unwrap() = Some((found.clone(), Instant::now()));
        found
    }

    /// resources\app beside Code.exe, else in a versioned folder under the
    /// install (the newest), where an update in progress can put it.
    pub fn app_dir(install: &Path) -> Option<PathBuf> {
        let beside = install.join("resources\\app");
        if beside.join("package.json").is_file() {
            return Some(beside);
        }
        std::fs::read_dir(install)
            .ok()?
            .flatten()
            .map(|e| e.path().join("resources\\app"))
            .filter(|p| p.join("package.json").is_file())
            .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
            .or(Some(beside))
    }

    static ICON: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

    /// VS Code's own tile image from its install.
    pub fn icon(app_dir: &Path) -> Option<String> {
        let png = app_dir.join("resources\\win32\\code_150x150.png");
        if let Some((p, url)) = ICON.lock().unwrap().as_ref() {
            if *p == png {
                return Some(url.clone());
            }
        }
        let bytes = std::fs::read(&png).ok()?;
        let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
        *ICON.lock().unwrap() = Some((png, url.clone()));
        Some(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(parse_version("1.147.0"), Some(vec![1, 147, 0]));
        assert_eq!(parse_version("1.148.0-insider"), Some(vec![1, 148, 0]));
        assert_eq!(parse_version("x"), None);
        let dir = std::env::temp_dir().join(format!("consus-launcher-vscode-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (v, old) in [("1.132.0", true), ("1.147.0", false), ("1.200.1", false), ("2.0.0", false)] {
            std::fs::write(dir.join("package.json"), format!(r#"{{ "version": "{v}" }}"#)).unwrap();
            assert_eq!(too_old(&dir).is_some(), old, "{v}");
        }
        std::fs::remove_file(dir.join("package.json")).unwrap();
        assert_eq!(too_old(&dir), None, "an unreadable version does not block");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// VS Code 1.147's Windows install keeps its app folder under a
    /// versioned folder, not beside Code.exe (seen 2026-11-19).
    #[test]
    #[cfg(windows)]
    fn the_app_folder_is_found_beside_code_exe_or_in_a_versioned_folder() {
        let install = std::env::temp_dir().join(format!("consus-launcher-vscode-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let versioned = install.join("8a14f0cc1b").join("resources").join("app");
        std::fs::create_dir_all(&versioned).unwrap();
        std::fs::write(versioned.join("package.json"), r#"{ "version": "1.147.0" }"#).unwrap();
        assert_eq!(windows::app_dir(&install), Some(versioned.clone()));
        assert_eq!(version(&versioned), Some(vec![1, 147, 0]));
        let beside = install.join("resources").join("app");
        std::fs::create_dir_all(&beside).unwrap();
        std::fs::write(beside.join("package.json"), r#"{ "version": "1.140.0" }"#).unwrap();
        assert_eq!(windows::app_dir(&install), Some(beside), "beside Code.exe wins when it is there");
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn the_command_carries_only_the_launchers_folders() {
        let home = Path::new("/Users/x");
        let bin = Path::new("/Applications/Visual Studio Code.app/Contents/MacOS/Code");
        let args = |c: Command| c.get_args().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<String>>();
        let data = crate::vscode::data_dir(home).to_string_lossy().into_owned();
        let ext = crate::vscode::extensions_dir(home).to_string_lossy().into_owned();
        assert_eq!(args(command(bin, home, None)), ["--user-data-dir", data.as_str(), "--extensions-dir", ext.as_str()]);
        let work = crate::claude_code::work_dir(home);
        let first = args(command(bin, home, Some(&work)));
        assert_eq!(first.last().map(String::as_str), work.to_str(), "a new instance opens on ~/Consus");
        assert!(VSCODE_SCRUB.matches("ELECTRON_RUN_AS_NODE") && VSCODE_SCRUB.matches("VSCODE_IPC_HOOK_CLI"));
        assert!(VSCODE_SCRUB.matches("ANTHROPIC_API_KEY") && VSCODE_SCRUB.matches("OPENAI_API_KEY") && VSCODE_SCRUB.matches("CONSUS_API_KEY"));
        assert!(!VSCODE_SCRUB.matches("PATH") && !VSCODE_SCRUB.matches("HOME"));
    }
}
