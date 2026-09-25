// Claude Desktop third-party (gateway) mode, user-space config library.
//
// Shape checked against a working install on macOS, 2026-09-22:
//   ~/Library/Application Support/Claude-3p/configLibrary/<uuid>.json
//   ~/Library/Application Support/Claude-3p/configLibrary/_meta.json  {appliedId, entries[{id,name}]}
//   ~/Library/Application Support/Claude-3p/claude_desktop_config.json {"deploymentMode":"3p"}
// Key names match the Consus portal's verified Claude Desktop template and
// the app's ADMX export. The
// credential helper is the launcher binary itself, reached through a link,
// so the key is read from the OS keychain and never written to a file.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::models::{self, Target};

const ENTRY_ID: &str = "6c3e2a4e-0b1d-4f7a-9e8c-5a1c0f2d3b47";
const ENTRY_NAME: &str = "Consus Launcher";
/// The models a Claude Desktop picker lists: Claude only, the target level
/// only, newest per family first and marked as that family's default.
pub fn select_models(models_json: &Value, t: &Target) -> Vec<Value> {
    let mut seen: Vec<String> = Vec::new();
    models::claude_models(models_json, t)
        .into_iter()
        .map(|m| {
            let first = !seen.contains(&m.family);
            if first {
                seen.push(m.family.clone());
            }
            let mut v = json!({
                "name": m.id,
                "labelOverride": m.label(t),
                "anthropicFamilyTier": m.family,
            });
            if first {
                v["isFamilyDefault"] = json!(true);
            }
            v
        })
        .collect()
}

/// Where Claude Desktop's gateway mode keeps its config. On Windows the app
/// reads %LOCALAPPDATA%\Claude-3p itself (checked in the app's code, 2026-09-25).
fn profile_dir(home: &Path) -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"))
            .join("Claude-3p")
    } else {
        home.join("Library/Application Support/Claude-3p")
    }
}

fn library_dir(home: &Path) -> PathBuf {
    profile_dir(home).join("configLibrary")
}

fn marker_json() -> Value {
    json!({ "deploymentMode": "3p" })
}

fn read_meta(lib: &Path) -> Value {
    fs::read_to_string(lib.join("_meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "appliedId": Value::Null, "entries": [] }))
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    let s = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    fs::write(path, s).map_err(|e| format!("{}: {e}", path.display()))
}

/// Links the helper name to this very binary. Same code identity as the one
/// that wrote the keychain item, so a signed build never prompts. Windows
/// gets a hard link (no admin rights needed), or a copy on another volume.
pub fn write_helper(path: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let _ = fs::remove_file(path);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&exe, path).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        // A helper a tool is running right now cannot be replaced; the one
        // in place is this same program, so keep it.
        if path.exists() {
            return Ok(());
        }
        fs::hard_link(&exe, path)
            .or_else(|_| fs::copy(&exe, path).map(|_| ()))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// A path as a command string a tool runs through its shell. Windows tools
/// may use Git Bash or cmd; forward slashes work in both.
pub fn command_path(p: &Path) -> String {
    let s = p.display().to_string();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s
    }
}

/// Adds (or refreshes) the launcher's own entry in the config library and
/// makes it the applied one. Never edits or removes anyone else's entries.
pub fn write_claude_desktop(home: &Path, helper: &Path, models: &Value, t: &Target) -> Result<(), String> {
    let picked = select_models(models, t);
    if picked.is_empty() {
        return Err(format!("No Claude {} models are available to this key.", t.tag()));
    }

    let profile = profile_dir(home);
    let lib = library_dir(home);
    fs::create_dir_all(&lib).map_err(|e| e.to_string())?;

    let marker = profile.join("claude_desktop_config.json");
    if !marker.exists() {
        write_json(&marker, &marker_json())?;
    }

    let entry = json!({
        "inferenceProvider": "gateway",
        "inferenceGatewayBaseUrl": t.endpoint,
        "inferenceGatewayAuthScheme": "x-api-key",
        "inferenceCredentialKind": "helper-script",
        "inferenceCredentialHelper": helper.to_string_lossy(),
        "chatTabEnabled": true,
        "inferenceModels": picked,
        "banner": {
            "enabled": true,
            "text": "Runs on the Consus gateway.",
            "backgroundColor": "#E4DCBE",
            "textColor": "#231B15",
            "linkUrl": "https://consus.io",
        },
    });
    write_json(&lib.join(format!("{ENTRY_ID}.json")), &entry)?;

    let meta_path = lib.join("_meta.json");
    let backup = lib.join("_meta.json.consus-bak");
    if meta_path.exists() && !backup.exists() {
        fs::copy(&meta_path, &backup).map_err(|e| e.to_string())?;
    }
    let mut meta = read_meta(&lib);
    let mut entries = meta["entries"].as_array().cloned().unwrap_or_default();
    if !entries.iter().any(|e| e["id"] == ENTRY_ID) {
        entries.push(json!({ "id": ENTRY_ID, "name": ENTRY_NAME }));
    }
    meta["entries"] = Value::Array(entries);
    meta["appliedId"] = json!(ENTRY_ID);
    write_json(&meta_path, &meta)
}

/// Removes the launcher's entry. If it was the applied one, the first
/// remaining entry becomes applied. If nothing remains and the mode marker
/// is the one the launcher wrote, that goes too, so a machine the launcher
/// switched into gateway mode returns to how it was. A library that holds
/// nothing of the launcher's is left as it is.
pub fn remove_claude_desktop(home: &Path) -> Result<(), String> {
    let lib = library_dir(home);
    let entry = lib.join(format!("{ENTRY_ID}.json"));
    let had_entry = entry.exists();
    let _ = fs::remove_file(&entry);
    let meta_path = lib.join("_meta.json");
    if !meta_path.exists() {
        return Ok(());
    }
    // Strictly: a file the app is midway through writing must not read as
    // empty and take everyone's entries with it.
    let mut meta = fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(Value::is_object)
        .ok_or_else(|| format!("{}: could not be read", meta_path.display()))?;
    let all = meta["entries"].as_array().cloned().unwrap_or_default();
    let listed = meta["appliedId"] == ENTRY_ID || all.iter().any(|e| e["id"] == ENTRY_ID);
    if !had_entry && !listed {
        return Ok(());
    }
    let entries: Vec<Value> = all.into_iter().filter(|e| e["id"] != ENTRY_ID).collect();
    let none_left = entries.is_empty();
    if listed {
        if meta["appliedId"] == ENTRY_ID {
            meta["appliedId"] = entries.first().map(|e| e["id"].clone()).unwrap_or(Value::Null);
        }
        meta["entries"] = Value::Array(entries);
        write_json(&meta_path, &meta)?;
    }

    if none_left {
        let marker = profile_dir(home).join("claude_desktop_config.json");
        let ours = fs::read_to_string(&marker)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|v| v == marker_json())
            .unwrap_or(false);
        if ours {
            let _ = fs::remove_file(&marker);
        }
    }
    Ok(())
}

// On Windows the library lives in %LOCALAPPDATA%, outside a test's home.
#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    fn library(tag: &str, meta: &str) -> (PathBuf, PathBuf) {
        let home = std::env::temp_dir().join(format!("consus-launcher-desktop-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let lib = library_dir(&home);
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("_meta.json"), meta).unwrap();
        (home, lib)
    }

    #[test]
    fn a_library_without_the_launchers_entry_is_left_alone() {
        let meta = r#"{ "appliedId": "theirs", "entries": [{ "id": "theirs" }] }"#;
        let (home, lib) = library("theirs", meta);
        remove_claude_desktop(&home).unwrap();
        assert_eq!(fs::read_to_string(lib.join("_meta.json")).unwrap(), meta);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unreadable_library_is_an_error_not_an_empty_one() {
        let meta = r#"{ "appliedId": "theirs", "entr"#;
        let (home, lib) = library("torn", meta);
        fs::write(lib.join(format!("{ENTRY_ID}.json")), "{}").unwrap();
        assert!(remove_claude_desktop(&home).is_err());
        assert_eq!(fs::read_to_string(lib.join("_meta.json")).unwrap(), meta);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn removing_the_launchers_entry_applies_the_next_one() {
        let meta = format!(r#"{{ "appliedId": "{ENTRY_ID}", "entries": [{{ "id": "{ENTRY_ID}" }}, {{ "id": "theirs" }}] }}"#);
        let (home, lib) = library("ours", &meta);
        fs::write(lib.join(format!("{ENTRY_ID}.json")), "{}").unwrap();
        remove_claude_desktop(&home).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(lib.join("_meta.json")).unwrap()).unwrap();
        assert_eq!(after, json!({ "appliedId": "theirs", "entries": [{ "id": "theirs" }] }));
        assert!(!lib.join(format!("{ENTRY_ID}.json")).exists());
        let _ = fs::remove_dir_all(&home);
    }
}
