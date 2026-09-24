// Claude Desktop third-party (gateway) mode, user-space config library.
//
// Shape checked against a working install on macOS, 2026-09-22:
//   ~/Library/Application Support/Claude-3p/configLibrary/<uuid>.json
//   ~/Library/Application Support/Claude-3p/configLibrary/_meta.json  {appliedId, entries[{id,name}]}
//   ~/Library/Application Support/Claude-3p/claude_desktop_config.json {"deploymentMode":"3p"}
// Key names match the portal's verified template (consus-key-portal,
// lib/deploy/templates/claudeDesktop.ts) and the app's ADMX export. The
// credential helper is the launcher binary itself, reached through a link,
// so the key is read from the OS keychain and never written to a file.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::models::{self, REGIME_TAG};

const ENTRY_ID: &str = "6c3e2a4e-0b1d-4f7a-9e8c-5a1c0f2d3b47";
const ENTRY_NAME: &str = "Consus Launcher";
const GATEWAY: &str = "https://api.consus.io";
/// The models a Claude Desktop picker lists: Claude only, this regime only,
/// newest per family first and marked as that family's default.
pub fn select_models(models_json: &Value) -> Vec<Value> {
    let mut seen: Vec<String> = Vec::new();
    models::claude_models(models_json)
        .into_iter()
        .map(|m| {
            let first = !seen.contains(&m.family);
            if first {
                seen.push(m.family.clone());
            }
            let mut v = json!({
                "name": m.id,
                "labelOverride": m.label(),
                "anthropicFamilyTier": m.family,
            });
            if first {
                v["isFamilyDefault"] = json!(true);
            }
            v
        })
        .collect()
}

fn profile_dir(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Claude-3p")
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
/// that wrote the keychain item, so a signed build never prompts.
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
        let _ = exe;
        Err("the key helper is not supported on this platform yet".to_string())
    }
}

/// Adds (or refreshes) the launcher's own entry in the config library and
/// makes it the applied one. Never edits or removes anyone else's entries.
pub fn write_claude_desktop(home: &Path, helper: &Path, models: &Value) -> Result<(), String> {
    let picked = select_models(models);
    if picked.is_empty() {
        return Err(format!("No Claude {REGIME_TAG} models are available to this key."));
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
        "inferenceGatewayBaseUrl": GATEWAY,
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
/// switched into gateway mode returns to how it was.
pub fn remove_claude_desktop(home: &Path) -> Result<(), String> {
    let lib = library_dir(home);
    let _ = fs::remove_file(lib.join(format!("{ENTRY_ID}.json")));
    let meta_path = lib.join("_meta.json");
    if !meta_path.exists() {
        return Ok(());
    }
    let mut meta = read_meta(&lib);
    let entries: Vec<Value> = meta["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e["id"] != ENTRY_ID)
        .collect();
    if meta["appliedId"] == ENTRY_ID {
        meta["appliedId"] = entries.first().map(|e| e["id"].clone()).unwrap_or(Value::Null);
    }
    let none_left = entries.is_empty();
    meta["entries"] = Value::Array(entries);
    write_json(&meta_path, &meta)?;

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
