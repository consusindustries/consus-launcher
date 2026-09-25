// Pi: an isolated agent directory at ~/.pi-consus-gateway, set through
// PI_CODING_AGENT_DIR, so the user's own ~/.pi is never touched.
//
// Docs checked: the Consus integration guide for Pi (its ITAR example is
//   templates/pi-models-itar.json, verbatim), and Pi 0.84.1's own
//   docs/models.md and core/resolve-config-value.js (2026-09-24).
//
// Two changes from the guide. The model list is trimmed to what this key can
// use. And x-api-key is not "$CONSUS_API_KEY" from the environment: it is
// "!<helper>", a command Pi runs at request time, so the key is not in the
// environment every command the agent runs inherits. The agent can still run
// the helper itself, as with Claude Code's apiKeyHelper: this keeps the key
// out of casual reach, it is not a boundary.
//
// models.json is merged by provider: only providers.consus is the launcher's.
// In settings.json the launcher sets defaultProvider and defaultModel, and
// only when the current default is not a Consus model this key still has,
// so a model the user picked in Pi survives the next launch.

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::claude_code::read_object;
use crate::models::{self, Target};

pub const PROFILE_DIR: &str = ".pi-consus-gateway";
const PROVIDER: &str = "consus";

// The guide's ITAR example. For another level the same models get that
// level's suffix and label, until the gateway reports each model's limits.
const TEMPLATE: &str = include_str!("../templates/pi-models-itar.json");

pub fn profile_dir(home: &Path) -> PathBuf {
    home.join(PROFILE_DIR)
}

fn models_path(home: &Path) -> PathBuf {
    profile_dir(home).join("models.json")
}

fn settings_path(home: &Path) -> PathBuf {
    profile_dir(home).join("settings.json")
}

/// Writes a temp file and renames it over the target, so an interrupted
/// write never leaves a truncated file that would block every later launch.
fn write_object(path: &Path, doc: Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&Value::Object(doc)).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text + "\n")
        .and_then(|_| fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })
}

/// The guide's provider block for this key, helper, and target.
fn provider(helper: &Path, models_json: &Value, t: &Target) -> Result<Value, String> {
    let have = models::ids(models_json);
    let tag = t.tag();
    let mut doc: Value = serde_json::from_str(TEMPLATE).expect("template is valid JSON");
    let mut p = doc["providers"][PROVIDER].take();
    let list = p["models"].as_array_mut().expect("template lists models");
    for m in list.iter_mut() {
        let base = m["id"].as_str().and_then(|id| id.strip_suffix(":itar")).map(String::from);
        let name = m["name"].as_str().and_then(|n| n.strip_suffix(" (ITAR)")).map(String::from);
        if let (Some(base), Some(name)) = (base, name) {
            m["id"] = json!(format!("{base}:{}", t.level));
            m["name"] = json!(format!("{name} ({tag})"));
        }
    }
    list.retain(|m| m["id"].as_str().is_some_and(|id| have.contains(&id)));
    if list.is_empty() {
        return Err(format!("No {tag} models for Pi are available to this key."));
    }
    p["baseUrl"] = json!(t.v1());
    // Pi runs this through the shell; the path can contain a space.
    p["headers"]["x-api-key"] = json!(format!("!\"{}\"", helper.display()));
    Ok(p)
}

pub fn write_config(home: &Path, helper: &Path, models_json: &Value, t: &Target) -> Result<(), String> {
    let p = provider(helper, models_json, t)?;
    let ids: Vec<String> = p["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(String::from))
        .collect();
    fs::create_dir_all(profile_dir(home)).map_err(|e| e.to_string())?;

    // Read both files before writing either, so a bad one changes nothing.
    let mpath = models_path(home);
    let spath = settings_path(home);
    let mut mdoc = read_object(&mpath)?;
    let mut sdoc = read_object(&spath)?;

    mdoc.entry("providers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| format!("{}: 'providers' is not an object", mpath.display()))?
        .insert(PROVIDER.into(), p);

    let keep = sdoc.get("defaultProvider").and_then(Value::as_str) == Some(PROVIDER)
        && sdoc.get("defaultModel").and_then(Value::as_str).is_some_and(|m| ids.iter().any(|id| id == m));
    if !keep {
        // Newest Opus when the key has one, else the guide's first model.
        let default = models::claude_models(models_json, t)
            .into_iter()
            .map(|m| m.id)
            .find(|id| ids.contains(id))
            .unwrap_or_else(|| ids[0].clone());
        sdoc.insert("defaultProvider".into(), json!(PROVIDER));
        sdoc.insert("defaultModel".into(), json!(default));
    }

    write_object(&mpath, mdoc)?;
    write_object(&spath, sdoc)
}

/// Removes the launcher's provider and a Consus default, and leaves
/// everything else, including the profile directory and its sessions.
pub fn remove_config(home: &Path) -> Result<(), String> {
    let mpath = models_path(home);
    if mpath.exists() {
        let mut doc = read_object(&mpath)?;
        let removed = doc
            .get_mut("providers")
            .and_then(Value::as_object_mut)
            .and_then(|p| p.remove(PROVIDER))
            .is_some();
        if removed {
            write_object(&mpath, doc)?;
        }
    }
    let spath = settings_path(home);
    if spath.exists() {
        let mut doc = read_object(&spath)?;
        if doc.get("defaultProvider").and_then(Value::as_str) == Some(PROVIDER) {
            doc.remove("defaultProvider");
            doc.remove("defaultModel");
            write_object(&spath, doc)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: &str = r#"[
        { "id": "consus/claude-opus-5:itar" },
        { "id": "consus/claude-opus-5-5:itar" },
        { "id": "consus/gpt-5.4:itar" },
        { "id": "consus/gpt-5.4:il5+itar" },
        { "id": "consus/gemini-3-flash:il4" }
    ]"#;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consus-launcher-pi-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn helper() -> PathBuf {
        PathBuf::from("/Users/x/Library/Application Support/io.consus.launcher/pi-key-helper")
    }

    fn read(p: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn writes_the_guide_block_trimmed_to_the_key() {
        let home = temp_home("write");
        write_config(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &Target::default()).unwrap();
        let p = &read(&models_path(&home))["providers"]["consus"];
        let ids: Vec<&str> = p["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["claude-opus-5:itar", "claude-opus-5-5:itar", "gpt-5.4:itar"]);
        assert_eq!(p["baseUrl"], "https://api.consus.io/v1");
        assert_eq!(p["compat"]["maxTokensField"], "max_tokens");
        assert_eq!(p["headers"]["x-api-key"], "!\"/Users/x/Library/Application Support/io.consus.launcher/pi-key-helper\"");
        let s = read(&settings_path(&home));
        assert_eq!(s["defaultProvider"], "consus");
        assert_eq!(s["defaultModel"], "claude-opus-5-5:itar");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn keeps_other_providers_and_the_users_model_pick() {
        let home = temp_home("merge");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        fs::write(models_path(&home), r#"{ "providers": { "local": { "baseUrl": "http://localhost" } } }"#).unwrap();
        fs::write(settings_path(&home), r#"{ "theme": "dark", "defaultProvider": "consus", "defaultModel": "gpt-5.4:itar" }"#).unwrap();
        write_config(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &Target::default()).unwrap();
        let m = read(&models_path(&home));
        assert_eq!(m["providers"]["local"]["baseUrl"], "http://localhost");
        assert!(m["providers"]["consus"].is_object());
        let s = read(&settings_path(&home));
        assert_eq!(s["theme"], "dark");
        assert_eq!(s["defaultModel"], "gpt-5.4:itar");

        remove_config(&home).unwrap();
        assert_eq!(read(&models_path(&home)), json!({ "providers": { "local": { "baseUrl": "http://localhost" } } }));
        assert_eq!(read(&settings_path(&home)), json!({ "theme": "dark" }));
        assert!(profile_dir(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unreadable_file_changes_nothing() {
        let home = temp_home("unreadable");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        let bad: &[u8] = b"{ \"theme\": \"dark\xff\" }";
        fs::write(settings_path(&home), bad).unwrap();
        assert!(write_config(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &Target::default()).is_err());
        assert_eq!(fs::read(settings_path(&home)).unwrap(), bad);
        assert!(!models_path(&home).exists(), "models.json must not be written either");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn no_itar_models_is_an_error() {
        let home = temp_home("empty");
        let err = write_config(&home, &helper(), &json!([{ "id": "consus/gemini-3-flash:il4" }]), &Target::default()).unwrap_err();
        assert!(err.contains("No ITAR models for Pi"));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn follows_the_target_level_and_endpoint() {
        let home = temp_home("target");
        let t = Target { endpoint: "https://ai-proxy.acme.example".into(), level: "fedramp-high".into() };
        let models = json!([{ "id": "consus/claude-opus-5-5:fedramp-high" }, { "id": "consus/claude-opus-5-5:itar" }]);
        write_config(&home, &helper(), &models, &t).unwrap();
        let p = &read(&models_path(&home))["providers"]["consus"];
        assert_eq!(p["baseUrl"], "https://ai-proxy.acme.example/v1");
        assert_eq!(p["models"].as_array().unwrap().len(), 1);
        assert_eq!(p["models"][0]["id"], "claude-opus-5-5:fedramp-high");
        assert_eq!(p["models"][0]["name"], "Claude Opus 5.5 (FedRAMP High)");
        assert_eq!(read(&settings_path(&home))["defaultModel"], "claude-opus-5-5:fedramp-high");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn every_template_model_can_take_another_level() {
        // The rewrite to another compliance level relies on these suffixes;
        // a row without them would keep :itar in a non-ITAR list.
        let doc: Value = serde_json::from_str(TEMPLATE).unwrap();
        for m in doc["providers"][PROVIDER]["models"].as_array().unwrap() {
            assert!(m["id"].as_str().unwrap().ends_with(":itar"), "{}", m["id"]);
            assert!(m["name"].as_str().unwrap().ends_with(" (ITAR)"), "{}", m["name"]);
        }
    }
}
