// Claude Desktop third-party (gateway) mode, user-space config library.
//
// Shape checked against a working install on macOS, 2026-09-22:
//   ~/Library/Application Support/Claude-3p/configLibrary/<uuid>.json
//   ~/Library/Application Support/Claude-3p/configLibrary/_meta.json  {appliedId, entries[{id,name}]}
//   ~/Library/Application Support/Claude-3p/claude_desktop_config.json {"deploymentMode":"3p"}
// Key names match the portal's verified template (consus-key-portal,
// lib/deploy/templates/claudeDesktop.ts) and the app's ADMX export. The
// credential is a helper script that reads the OS keychain, so the key is
// never written to a file.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const ENTRY_ID: &str = "6c3e2a4e-0b1d-4f7a-9e8c-5a1c0f2d3b47";
const ENTRY_NAME: &str = "Consus Launcher";
const GATEWAY: &str = "https://api.consus.io";
const REGIME: &str = "itar";
const REGIME_TAG: &str = "ITAR";
const FAMILIES: [&str; 3] = ["opus", "sonnet", "haiku"];

pub const HELPER_SCRIPT: &str = r#"#!/bin/sh
# Consus Launcher: hands Claude Desktop the key from the macOS keychain.
K="$(security find-generic-password -s io.consus.launcher -a default -w 2>/dev/null)"
[ -n "$K" ] || { echo "No Consus key in the keychain" >&2; exit 1; }
printf '{"token":"%s","headers":{"x-api-key":"%s"}}' "$K" "$K"
"#;

struct ClaudeModel {
    name: String,
    family: String,
    version: Vec<u32>,
    label: String,
}

// "consus/claude-opus-4-8:itar" -> name "claude-opus-4-8:itar", label "Opus 4.8 ITAR"
fn parse(id: &str) -> Option<ClaudeModel> {
    let bare = id.strip_prefix("consus/").unwrap_or(id);
    let (base, suffix) = bare.split_once(':')?;
    if suffix != REGIME {
        return None;
    }
    let mut parts = base.strip_prefix("claude-")?.split('-');
    let family = parts.next()?.to_string();
    if !FAMILIES.contains(&family.as_str()) {
        return None;
    }
    let version: Vec<u32> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let mut fam = family.clone();
    fam.replace_range(..1, &family[..1].to_uppercase());
    let ver = version.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
    Some(ClaudeModel {
        name: bare.to_string(),
        family,
        version,
        label: format!("{fam} {ver} {REGIME_TAG}"),
    })
}

/// The models a Claude Desktop picker lists: Claude only, this regime only,
/// newest per family first and marked as that family's default.
pub fn select_models(models: &Value) -> Vec<Value> {
    let mut picked: Vec<ClaudeModel> = models
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m.get("owned_by").and_then(Value::as_str) == Some("anthropic"))
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .filter_map(parse)
        .collect();
    let rank = |f: &str| FAMILIES.iter().position(|x| *x == f).unwrap_or(9);
    picked.sort_by(|a, b| rank(&a.family).cmp(&rank(&b.family)).then(b.version.cmp(&a.version)));

    let mut seen: Vec<String> = Vec::new();
    picked
        .into_iter()
        .map(|m| {
            let first = !seen.contains(&m.family);
            if first {
                seen.push(m.family.clone());
            }
            let mut v = json!({
                "name": m.name,
                "labelOverride": m.label,
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

fn read_meta(lib: &Path) -> Value {
    fs::read_to_string(lib.join("_meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({ "appliedId": Value::Null, "entries": [] }))
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    let s = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    fs::write(path, s).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_helper(path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(path, HELPER_SCRIPT).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Adds (or refreshes) the launcher's own entry in the config library and
/// makes it the applied one. Never edits or removes anyone else's entries.
pub fn write_claude_desktop(home: &Path, helper: &Path, models: &Value) -> Result<(), String> {
    let profile = profile_dir(home);
    let lib = library_dir(home);
    fs::create_dir_all(&lib).map_err(|e| e.to_string())?;

    let marker = profile.join("claude_desktop_config.json");
    if !marker.exists() {
        write_json(&marker, &json!({ "deploymentMode": "3p" }))?;
    }

    let entry = json!({
        "inferenceProvider": "gateway",
        "inferenceGatewayBaseUrl": GATEWAY,
        "inferenceGatewayAuthScheme": "x-api-key",
        "inferenceCredentialKind": "helper-script",
        "inferenceCredentialHelper": helper.to_string_lossy(),
        "chatTabEnabled": true,
        "inferenceModels": select_models(models),
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
    let entries = meta["entries"].as_array().cloned().unwrap_or_default();
    if !entries.iter().any(|e| e["id"] == ENTRY_ID) {
        let mut entries = entries;
        entries.push(json!({ "id": ENTRY_ID, "name": ENTRY_NAME }));
        meta["entries"] = Value::Array(entries);
    }
    meta["appliedId"] = json!(ENTRY_ID);
    write_json(&meta_path, &meta)
}

/// Removes the launcher's entry. If it was the applied one, the first
/// remaining entry becomes applied so the app is not left pointing at nothing.
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
    meta["entries"] = Value::Array(entries);
    write_json(&meta_path, &meta)
}
