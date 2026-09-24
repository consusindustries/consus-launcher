// Codex CLI: an isolated home at ~/.codex-consus-gateway, set through
// CODEX_HOME as the Consus guide recommends, so the user's own ~/.codex is
// never touched.
//
// Docs checked: gov_open_router/docs/integrations/codex.md (2026-09-24),
//   against Codex CLI 0.147.
//
// config.toml is the ChatGPT desktop template, the same compliance baseline
// (see chatgpt.rs), merged key by key the same way, plus three keys only
// Codex needs: the model catalog, no update check on startup, and trust for
// the empty working folder so the first run does not stop to ask.
//
// The catalog is the guide's make-catalog.py: each Consus id is cloned from
// the installed Codex's own bundled entry (those carry version-specific
// instructions, so it is rebuilt on every launch) with the gateway's context
// window and reasoning efforts. Without it /model cannot list Consus models
// and Codex assumes a small context window. ITAR rows only.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, Table};

use crate::{chatgpt, claude_code, models};

pub const PROFILE_DIR: &str = ".codex-consus-gateway";

const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh"];

// Consus id, bundled entry it is cloned from, name, context window, efforts.
// From the guide's MODELS table.
const CATALOG: [(&str, &str, &str, u64, &[&str]); 5] = [
    ("gpt-5.6-sol:itar", "gpt-5.6-sol", "GPT-5.6 Sol (ITAR)", 922_000, EFFORTS),
    ("gpt-5.6-terra:itar", "gpt-5.6-terra", "GPT-5.6 Terra (ITAR)", 1_000_000, EFFORTS),
    ("gpt-5.6-luna:itar", "gpt-5.6-luna", "GPT-5.6 Luna (ITAR)", 1_000_000, EFFORTS),
    ("gpt-5.4:itar", "gpt-5.4", "GPT-5.4 (ITAR)", 272_000, EFFORTS),
    ("gpt-5.1:itar", "gpt-5.4", "GPT-5.1 (ITAR)", 272_000, &["low", "medium", "high"]),
];

pub fn profile_dir(home: &Path) -> PathBuf {
    home.join(PROFILE_DIR)
}

fn config_path(home: &Path) -> PathBuf {
    profile_dir(home).join("config.toml")
}

fn catalog_path(home: &Path) -> PathBuf {
    profile_dir(home).join("consus-models.json")
}

/// The catalog entries for this key, from `codex debug models --bundled`.
pub fn catalog(bundled: &Value, models_json: &Value) -> Vec<Value> {
    let have = models::ids(models_json);
    let stock = bundled["models"].as_array().cloned().unwrap_or_default();
    CATALOG
        .iter()
        .filter(|(id, ..)| have.contains(id))
        .filter_map(|(id, from, name, ctx, efforts)| {
            let mut m = stock.iter().find(|m| m["slug"] == *from)?.clone();
            let levels: Vec<Value> = m["supported_reasoning_levels"]
                .as_array()?
                .iter()
                .filter(|l| l["effort"].as_str().is_some_and(|e| efforts.contains(&e)))
                .cloned()
                .collect();
            let o = m.as_object_mut()?;
            o.insert("slug".into(), json!(id));
            o.insert("display_name".into(), json!(name));
            o.insert("description".into(), json!("Consus Gateway"));
            o.insert("visibility".into(), json!("list"));
            o.insert("context_window".into(), json!(ctx));
            o.insert("max_context_window".into(), json!(ctx));
            o.insert("supported_reasoning_levels".into(), json!(levels));
            o.insert("default_reasoning_level".into(), json!("medium"));
            o.insert("additional_speed_tiers".into(), json!([]));
            o.insert("service_tiers".into(), json!([]));
            o.insert("availability_nux".into(), Value::Null);
            o.insert("upgrade".into(), Value::Null);
            o.insert("supports_search_tool".into(), json!(false));
            Some(m)
        })
        .enumerate()
        .map(|(i, mut m)| {
            m["priority"] = json!(i + 1);
            m
        })
        .collect()
}

/// The keys only Codex's file carries.
fn extra(home: &Path, with_catalog: bool) -> Table {
    let mut t = Table::new();
    if with_catalog {
        t.insert("model_catalog_json", value(catalog_path(home).display().to_string()));
    }
    t.insert("check_for_update_on_startup", value(false));
    let mut trust = Table::new();
    trust.insert("trust_level", value("trusted"));
    let mut projects = Table::new();
    projects.set_implicit(true);
    projects.insert(&claude_code::work_dir(home).display().to_string(), Item::Table(trust));
    t.insert("projects", Item::Table(projects));
    t
}

/// `bundled` is None when the installed Codex could not list its models;
/// Codex then runs without a catalog rather than not at all.
pub fn write_config(home: &Path, models_json: &Value, bundled: Option<&Value>) -> Result<(), String> {
    fs::create_dir_all(profile_dir(home)).map_err(|e| e.to_string())?;
    let list = bundled.map(|b| catalog(b, models_json)).unwrap_or_default();
    let cpath = catalog_path(home);
    if !list.is_empty() {
        let text = serde_json::to_string_pretty(&json!({ "models": list })).map_err(|e| e.to_string())?;
        let tmp = cpath.with_extension("json.tmp");
        fs::write(&tmp, text + "\n")
            .and_then(|_| fs::rename(&tmp, &cpath))
            .map_err(|e| format!("{}: {e}", cpath.display()))?;
    }
    // Codex refuses to start when model_catalog_json names a missing file.
    let with_catalog = cpath.exists();
    let path = config_path(home);
    chatgpt::write_config_at(&path, models_json, &extra(home, with_catalog))?;
    if !with_catalog {
        let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let mut doc: DocumentMut = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
        if doc.remove("model_catalog_json").is_some() {
            fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Removes the launcher's keys and its catalog, and leaves everything else,
/// including the profile directory and its sessions.
pub fn remove_config(home: &Path) -> Result<(), String> {
    chatgpt::remove_config_at(&config_path(home), &extra(home, true))?;
    match fs::remove_file(catalog_path(home)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: &str = r#"[
        { "id": "consus/gpt-5.6-terra:itar" },
        { "id": "consus/gpt-5.1:itar" },
        { "id": "consus/gpt-4.1:il5+itar" },
        { "id": "consus/claude-opus-5-5:itar" }
    ]"#;

    fn bundled() -> Value {
        let level = |e: &str| json!({ "effort": e, "description": e });
        json!({ "models": [
            { "slug": "gpt-5.6-terra", "display_name": "GPT-5.6 Terra", "base_instructions": "terra",
              "supported_reasoning_levels": [level("minimal"), level("low"), level("medium"), level("high"), level("xhigh")] },
            { "slug": "gpt-5.4", "display_name": "GPT-5.4", "base_instructions": "5.4",
              "supported_reasoning_levels": [level("low"), level("medium"), level("high"), level("xhigh")] }
        ]})
    }

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consus-launcher-codex-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn catalog_clones_bundled_entries_for_the_keys_itar_models() {
        let list = catalog(&bundled(), &serde_json::from_str(MODELS).unwrap());
        let slugs: Vec<&str> = list.iter().map(|m| m["slug"].as_str().unwrap()).collect();
        assert_eq!(slugs, ["gpt-5.6-terra:itar", "gpt-5.1:itar"]);
        assert_eq!(list[0]["base_instructions"], "terra");
        assert_eq!(list[0]["context_window"], 1_000_000);
        assert_eq!(list[0]["priority"], 1);
        assert_eq!(list[1]["base_instructions"], "5.4", "5.1 is cloned from 5.4, as in the guide");
        let efforts: Vec<&str> =
            list[1]["supported_reasoning_levels"].as_array().unwrap().iter().map(|l| l["effort"].as_str().unwrap()).collect();
        assert_eq!(efforts, ["low", "medium", "high"]);
    }

    #[test]
    fn writes_template_and_codex_keys_then_removes_only_those() {
        let home = temp_home("write");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        fs::write(config_path(&home), "notify = [\"x\"]\n\n[projects.\"/Users/x/proj\"]\ntrust_level = \"trusted\"\n").unwrap();
        write_config(&home, &serde_json::from_str(MODELS).unwrap(), Some(&bundled())).unwrap();

        let doc: DocumentMut = fs::read_to_string(config_path(&home)).unwrap().parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("gpt-5.6-terra:itar"));
        assert_eq!(doc["model_provider"].as_str(), Some("consus"));
        assert_eq!(doc["check_for_update_on_startup"].as_bool(), Some(false));
        assert_eq!(doc["model_catalog_json"].as_str(), Some(catalog_path(&home).to_str().unwrap()));
        let work = claude_code::work_dir(&home).display().to_string();
        assert_eq!(doc["projects"][work.as_str()]["trust_level"].as_str(), Some("trusted"));
        assert!(catalog_path(&home).exists());

        remove_config(&home).unwrap();
        let doc: DocumentMut = fs::read_to_string(config_path(&home)).unwrap().parse().unwrap();
        assert!(doc.get("model_provider").is_none() && doc.get("check_for_update_on_startup").is_none());
        assert!(doc["projects"].get(work.as_str()).is_none());
        assert_eq!(doc["projects"]["/Users/x/proj"]["trust_level"].as_str(), Some("trusted"));
        assert!(doc.get("notify").is_some());
        assert!(!catalog_path(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn no_catalog_means_no_catalog_key() {
        let home = temp_home("nocatalog");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        let mut old = DocumentMut::new();
        old["model_catalog_json"] = value(catalog_path(&home).display().to_string());
        fs::write(config_path(&home), old.to_string()).unwrap();
        write_config(&home, &serde_json::from_str(MODELS).unwrap(), None).unwrap();
        let doc: DocumentMut = fs::read_to_string(config_path(&home)).unwrap().parse().unwrap();
        assert!(doc.get("model_catalog_json").is_none());
        assert_eq!(doc["model"].as_str(), Some("gpt-5.6-terra:itar"));
        let _ = fs::remove_dir_all(&home);
    }
}
