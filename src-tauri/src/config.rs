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

/// The mode marker that switches Claude Desktop into gateway mode.
fn marker_path(home: &Path) -> PathBuf {
    profile_dir(home).join("claude_desktop_config.json")
}

/// Left beside the marker when the launcher creates it. The app rewrites the
/// marker with its own preferences on first run, so its content cannot say
/// whose it is; this file can, and sign out uses it to switch the app back.
fn created_path(home: &Path) -> PathBuf {
    profile_dir(home).join("claude_desktop_config.json.consus-created")
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
    #[cfg(unix)]
    {
        // Already this binary: left as it is, so a tool running the helper
        // at this moment never finds it missing.
        if fs::read_link(path).is_ok_and(|t| t == exe) {
            return Ok(());
        }
        // Made beside it and moved over it in one step, for the same reason.
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = path.with_file_name(format!(".{name}.{}", std::process::id()));
        let _ = fs::remove_file(&tmp);
        std::os::unix::fs::symlink(&exe, &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })
    }
    #[cfg(not(unix))]
    {
        let _ = fs::remove_file(path);
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

/// The helper as a whole shell command (the policy's `helper_shell` slot):
/// quoted, since the path can contain a space.
pub fn helper_shell(helper: &Path) -> String {
    format!("\"{}\"", command_path(helper))
}

fn no_models_message(t: &Target) -> String {
    format!("No Claude {} models are available to this key.", t.tag())
}

/// Why Claude Desktop cannot be set up with this key, if it cannot.
pub fn no_models(models: &Value, t: &Target) -> Option<String> {
    select_models(models, t).is_empty().then(|| no_models_message(t))
}

/// Adds (or refreshes) the launcher's own entry in the config library and
/// makes it the applied one. Never edits or removes anyone else's entries.
pub fn write_claude_desktop(home: &Path, helper: &Path, models: &Value, t: &Target) -> Result<(), String> {
    let picked = select_models(models, t);
    if picked.is_empty() {
        return Err(no_models_message(t));
    }

    let lib = library_dir(home);
    fs::create_dir_all(&lib).map_err(|e| e.to_string())?;

    // A marker already there was not the launcher's: it stays at sign out.
    let marker = marker_path(home);
    if !marker.exists() {
        write_json(&marker, &marker_json())?;
        fs::write(created_path(home), "").map_err(|e| e.to_string())?;
    }

    // The entry's values come from the policy document (policy/
    // launcher-policy.json): the static keys that keep the app off
    // Anthropic's network (telemetry, update checks, the model catalog poll,
    // the WebFetch hostname check, plugin marketplaces, Claude.ai sign-in),
    // then the web-access variant (the agent's web tools and what its
    // sandbox may reach: unset or [] refuses every host, ["*"] allows all).
    // What stays is the VM bundle from downloads.claude.ai at the first
    // Cowork or Code session, which only Anthropic's offline installer
    // avoids. An app older than the one that added a key ignores it
    // (modelCatalogEnabled 2.110.0, skipWebFetchPreflight 1.37937.0, the
    // plugin keys 1.37937.0, the sandbox keys 1.2581.0). Only the three
    // slots are filled here.
    let policy = crate::policy::policy();
    let mut entry = policy.claude_desktop_static();
    entry.insert("inferenceGatewayBaseUrl".into(), json!(t.endpoint));
    entry.insert("inferenceCredentialHelper".into(), json!(helper.to_string_lossy()));
    entry.insert("inferenceModels".into(), json!(picked));
    entry.extend(policy.claude_desktop_variant(t.web_access));
    // The connectors the org turned on (Slack: the org's Slack app, with
    // its write tools blocked). The entry is rewritten whole, so one the
    // org turned off is gone at the next launch, and so is one a person
    // added to this entry by hand.
    let servers: Vec<Value> = t.connectors.iter().filter_map(|c| policy.connector_entry(c, "claude-desktop")).collect();
    if !servers.is_empty() {
        entry.insert("managedMcpServers".into(), Value::Array(servers));
    }
    write_json(&lib.join(format!("{ENTRY_ID}.json")), &Value::Object(entry))?;

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

/// Whether the launcher has anything set up in Claude Desktop's gateway
/// profile, so sign out has something to remove (and Claude, if open, must
/// be closed first).
pub fn has_claude_desktop(home: &Path) -> bool {
    let lib = library_dir(home);
    lib.join(format!("{ENTRY_ID}.json")).exists()
        || created_path(home).exists()
        || read_meta(&lib)["entries"].as_array().is_some_and(|e| e.iter().any(|e| e["id"] == ENTRY_ID))
}

/// Removes the launcher's entry. If it was the applied one, the first
/// remaining entry becomes applied. If nothing remains and the launcher
/// created the mode marker, that goes too, whatever the app has since added
/// to it, so a machine the launcher switched into gateway mode returns to
/// how it was. A marker the launcher found already there stays, and so does
/// one other entries still need, which is theirs from then on. A library
/// that holds nothing of the launcher's is left as it is.
pub fn remove_claude_desktop(home: &Path) -> Result<(), String> {
    let lib = library_dir(home);
    let entry = lib.join(format!("{ENTRY_ID}.json"));
    let had_entry = entry.exists();
    let _ = fs::remove_file(&entry);
    // The record outlives a marker removal that failed, so a later sign out
    // still reaches the marker after the entry is already gone.
    let created = created_path(home).exists();
    let meta_path = lib.join("_meta.json");
    let none_left = if meta_path.exists() {
        // Strictly: a file the app is midway through writing must not read
        // as empty and take everyone's entries with it.
        let mut meta = fs::read_to_string(&meta_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(Value::is_object)
            .ok_or_else(|| format!("{}: could not be read", meta_path.display()))?;
        let all = meta["entries"].as_array().cloned().unwrap_or_default();
        let listed = meta["appliedId"] == ENTRY_ID || all.iter().any(|e| e["id"] == ENTRY_ID);
        if !had_entry && !listed && !created {
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
        none_left
    } else if created {
        true
    } else {
        return Ok(());
    };

    if !none_left {
        // Other entries still run in gateway mode, so the marker is theirs
        // from now on and stays at every later sign out too.
        let _ = fs::remove_file(created_path(home));
        return Ok(());
    }
    let marker = marker_path(home);
    // Without the record (a launcher before 1.5.1 made it), only a marker
    // still exactly as the launcher wrote it is known to be ours.
    let ours = created
        || fs::read_to_string(&marker)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .is_some_and(|v| v == marker_json());
    if ours {
        match fs::remove_file(&marker) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // The record stays, so the next sign out tries again.
            Err(e) => return Err(format!("{}: {e}", marker.display())),
        }
        let _ = fs::remove_file(created_path(home));
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
    fn the_entry_keeps_the_app_off_anthropics_network() {
        let (home, lib) = library("entry", r#"{ "appliedId": null, "entries": [] }"#);
        let models = json!([{ "id": "consus/claude-sonnet-5:itar" }]);
        write_claude_desktop(&home, Path::new("/x/helper"), &models, &Target::default()).unwrap();
        let entry: Value = serde_json::from_str(&fs::read_to_string(lib.join(format!("{ENTRY_ID}.json"))).unwrap()).unwrap();
        for k in [
            "disableEssentialTelemetry",
            "disableNonessentialTelemetry",
            "disableNonessentialServices",
            "disableAutoUpdates",
            "disableDeploymentModeChooser",
            "skipWebFetchPreflight",
        ] {
            assert_eq!(entry[k], true, "{k}");
        }
        for k in ["modelCatalogEnabled", "userPluginMarketplacesEnabled", "userPluginUploadsEnabled"] {
            assert_eq!(entry[k], false, "{k}");
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_entry_is_exactly_the_policy_documents_keys() {
        let p = crate::policy::policy();
        let cd = p.tool("claude-desktop");
        let models = json!([{ "id": "consus/claude-sonnet-5:itar" }]);
        for on in [false, true] {
            let (home, lib) = library(if on { "doc-on" } else { "doc-off" }, r#"{ "appliedId": null, "entries": [] }"#);
            write_claude_desktop(&home, Path::new("/x/helper"), &models, &Target { web_access: on, ..Target::default() }).unwrap();
            let entry: Value = serde_json::from_str(&fs::read_to_string(lib.join(format!("{ENTRY_ID}.json"))).unwrap()).unwrap();
            let mut written: Vec<&String> = entry.as_object().unwrap().keys().collect();
            written.sort();
            let variant = p.claude_desktop_variant(on);
            let mut expected: Vec<&String> = cd.settings.keys().chain(cd.slot_keys.keys()).chain(variant.keys()).collect();
            expected.sort();
            assert_eq!(written, expected, "web access {on}: the entry's keys are the document's");
            for (k, v) in &cd.settings {
                assert_eq!(&entry[k], v, "{k}");
            }
            for (k, v) in &variant {
                assert_eq!(&entry[k], v, "{k}");
            }
            let _ = fs::remove_dir_all(&home);
        }
        assert_eq!(cd.slot_keys["inferenceGatewayBaseUrl"], "endpoint");
        assert_eq!(cd.slot_keys["inferenceCredentialHelper"], "helper");
        assert_eq!(cd.slot_keys["inferenceModels"], "models");
    }

    #[test]
    fn web_access_is_closed_by_default_and_opened_by_the_org() {
        let models = json!([{ "id": "consus/claude-sonnet-5:itar" }]);
        let entry_for = |tag: &str, t: &Target| {
            let (home, lib) = library(tag, r#"{ "appliedId": null, "entries": [] }"#);
            write_claude_desktop(&home, Path::new("/x/helper"), &models, t).unwrap();
            let e: Value = serde_json::from_str(&fs::read_to_string(lib.join(format!("{ENTRY_ID}.json"))).unwrap()).unwrap();
            let _ = fs::remove_dir_all(&home);
            e
        };
        let off = entry_for("web-off", &Target::default());
        assert_eq!(off["disabledBuiltinTools"], json!(["WebSearch", "WebFetch"]));
        assert_eq!(off["coworkEgressAllowedHosts"], json!([]), "the agent's shell reaches no host either");
        let on = entry_for("web-on", &Target { web_access: true, ..Target::default() });
        assert_eq!(on["disabledBuiltinTools"], json!(["WebSearch"]), "search is a server tool the gateway refuses");
        assert_eq!(on["coworkEgressAllowedHosts"], json!(["*"]));
    }

    #[test]
    fn slack_is_in_the_entry_while_the_org_has_it_on() {
        let models = json!([{ "id": "consus/claude-sonnet-5:itar" }]);
        let (home, lib) = library("slack", r#"{ "appliedId": null, "entries": [] }"#);
        let read = || -> Value { serde_json::from_str(&fs::read_to_string(lib.join(format!("{ENTRY_ID}.json"))).unwrap()).unwrap() };
        let on = Target { connectors: vec![crate::models::slack("1234567.7654321")], ..Target::default() };
        write_claude_desktop(&home, Path::new("/x/helper"), &models, &on).unwrap();
        let servers = read()["managedMcpServers"].as_array().cloned().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0], crate::policy::policy().connector_entry(&on.connectors[0], "claude-desktop").unwrap());
        assert_eq!(servers[0]["oauth"]["clientId"], "1234567.7654321");
        write_claude_desktop(&home, Path::new("/x/helper"), &models, &Target::default()).unwrap();
        assert!(read().get("managedMcpServers").is_none(), "turned off in the portal: gone at the next launch");
        let _ = fs::remove_dir_all(&home);
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

    const MODELS: &str = r#"[{ "id": "consus/claude-sonnet-5:itar" }]"#;
    /// Claude Desktop rewrites the marker with its own preferences on first
    /// run (seen 2026-11-12), so it no longer equals what the launcher wrote.
    const REWRITTEN: &str = r#"{ "deploymentMode": "3p", "preferences": { "sidebarMode": "chat" } }"#;

    fn set_up(home: &Path) {
        let models: Value = serde_json::from_str(MODELS).unwrap();
        write_claude_desktop(home, Path::new("/x/helper"), &models, &Target::default()).unwrap();
    }

    #[test]
    fn the_launcher_records_a_marker_it_creates_and_only_that() {
        let (home, _) = library("record", r#"{ "appliedId": null, "entries": [] }"#);
        set_up(&home);
        assert!(marker_path(&home).exists());
        assert!(created_path(&home).exists());
        let _ = fs::remove_dir_all(&home);

        let (home, _) = library("found", r#"{ "appliedId": null, "entries": [] }"#);
        fs::write(marker_path(&home), REWRITTEN).unwrap();
        set_up(&home);
        assert!(!created_path(&home).exists(), "a marker already there is not the launcher's");
        assert_eq!(fs::read_to_string(marker_path(&home)).unwrap(), REWRITTEN);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn sign_out_removes_a_marker_the_launcher_created_after_the_app_rewrote_it() {
        let (home, _) = library("rewritten", r#"{ "appliedId": null, "entries": [] }"#);
        set_up(&home);
        assert!(has_claude_desktop(&home));
        fs::write(marker_path(&home), REWRITTEN).unwrap();
        remove_claude_desktop(&home).unwrap();
        assert!(!marker_path(&home).exists(), "the app is back to its normal mode");
        assert!(!created_path(&home).exists());
        assert!(!has_claude_desktop(&home));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn without_the_record_only_a_pristine_marker_goes() {
        let meta = format!(r#"{{ "appliedId": "{ENTRY_ID}", "entries": [{{ "id": "{ENTRY_ID}" }}] }}"#);
        let (home, lib) = library("old-rewritten", &meta);
        fs::write(lib.join(format!("{ENTRY_ID}.json")), "{}").unwrap();
        fs::write(marker_path(&home), REWRITTEN).unwrap();
        remove_claude_desktop(&home).unwrap();
        assert_eq!(fs::read_to_string(marker_path(&home)).unwrap(), REWRITTEN, "not known to be ours");
        let _ = fs::remove_dir_all(&home);

        let (home, lib) = library("old-pristine", &meta);
        fs::write(lib.join(format!("{ENTRY_ID}.json")), "{}").unwrap();
        write_json(&marker_path(&home), &marker_json()).unwrap();
        remove_claude_desktop(&home).unwrap();
        assert!(!marker_path(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn with_other_entries_left_the_marker_stays_and_becomes_theirs() {
        let (home, lib) = library("others", r#"{ "appliedId": "theirs", "entries": [{ "id": "theirs" }] }"#);
        set_up(&home);
        remove_claude_desktop(&home).unwrap();
        assert!(marker_path(&home).exists(), "their entry still runs in gateway mode");
        assert!(!created_path(&home).exists(), "the marker is theirs from now on");
        assert!(!has_claude_desktop(&home), "so a later sign out leaves Claude open");
        let after: Value = serde_json::from_str(&fs::read_to_string(lib.join("_meta.json")).unwrap()).unwrap();
        assert_eq!(after, json!({ "appliedId": "theirs", "entries": [{ "id": "theirs" }] }));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_marker_that_could_not_be_removed_is_tried_again_at_the_next_sign_out() {
        use std::os::unix::fs::PermissionsExt;
        let (home, _) = library("retry", r#"{ "appliedId": null, "entries": [] }"#);
        set_up(&home);
        let profile = profile_dir(&home);
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o555)).unwrap();
        let first = remove_claude_desktop(&home);
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o755)).unwrap();
        // Root ignores the mode, so the removal can succeed under it.
        if first.is_err() {
            assert!(marker_path(&home).exists());
            assert!(created_path(&home).exists(), "the record stays for the retry");
            assert!(has_claude_desktop(&home));
            remove_claude_desktop(&home).unwrap();
        }
        assert!(!marker_path(&home).exists());
        assert!(!created_path(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }
}
