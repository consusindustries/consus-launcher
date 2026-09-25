// Pi: an isolated agent directory at ~/.pi-consus-gateway, set through
// PI_CODING_AGENT_DIR, so the user's own ~/.pi is never touched.
//
// Docs checked: the Consus integration guide for Pi (the provider settings
//   below are its example's), and Pi 0.84.1's own docs/models.md and
//   core/resolve-config-value.js (2026-09-24).
//
// Two changes from the guide. The model list is built from what the gateway
// reports for this key at the org's level (names, limits, reasoning,
// pricing), not copied from the guide's ITAR table. And x-api-key is not
// "$CONSUS_API_KEY" from the environment: it is
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

/// Pi's reasoning levels, mapped onto the efforts a model takes; a level the
/// model lacks is null, so Pi does not offer it. "minimal" uses the lowest.
fn thinking_map(efforts: &[String]) -> Value {
    let has = |e: &str| efforts.iter().any(|x| x == e).then(|| e.to_string());
    json!({
        "off": "none",
        "minimal": efforts.first(),
        "low": has("low"),
        "medium": has("medium"),
        "high": has("high"),
        "xhigh": has("xhigh"),
    })
}

/// Claude first, then GPT, Gemini, Grok, and the rest, by name within each.
fn maker_rank(maker: &str) -> usize {
    ["anthropic", "openai", "google", "xai"].iter().position(|m| *m == maker).unwrap_or(4)
}

/// The guide's provider block, with one row per model this key has at the
/// target level.
fn provider(helper: &Path, models_json: &Value, t: &Target) -> Result<Value, String> {
    let tag = t.tag();
    let mut list = models::at_level(models_json, t);
    if list.is_empty() {
        return Err(format!("No {tag} models for Pi are available to this key."));
    }
    list.sort_by(|(_, a), (_, b)| maker_rank(&a.maker).cmp(&maker_rank(&b.maker)).then(a.name.cmp(&b.name)));
    let rows: Vec<Value> = list
        .into_iter()
        .map(|(id, d)| {
            let mut m = json!({
                "id": id,
                "name": format!("{} ({tag})", d.name),
                "reasoning": !d.efforts.is_empty(),
                "input": if d.image { json!(["text", "image"]) } else { json!(["text"]) },
                "contextWindow": d.context,
                "maxTokens": d.max_output,
                "cost": {
                    "input": d.pricing[0],
                    "output": d.pricing[1],
                    "cacheRead": d.pricing[2],
                    "cacheWrite": d.pricing[3],
                },
            });
            if !d.efforts.is_empty() {
                m["thinkingLevelMap"] = thinking_map(&d.efforts);
            }
            m
        })
        .collect();
    Ok(json!({
        "baseUrl": t.v1(),
        "api": "openai-completions",
        "apiKey": "consus",
        // Pi runs this through the shell; the path can contain a space.
        "headers": { "x-api-key": format!("!\"{}\"", crate::config::command_path(helper)) },
        "compat": {
            "supportsDeveloperRole": false,
            "supportsReasoningEffort": true,
            "thinkingFormat": "openai",
            "supportsStore": false,
            "supportsStrictMode": false,
            "maxTokensField": "max_tokens",
        },
        "models": rows,
    }))
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
        // Newest Opus when the key has one, else the first model.
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
        let models = json!([
            { "id": "consus/claude-opus-5-5:fedramp-high", "display_name": "Claude Opus 5.5" },
            { "id": "consus/claude-opus-5-5:itar", "display_name": "Claude Opus 5.5" }
        ]);
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
    fn rows_carry_the_gateways_limits_reasoning_and_pricing() {
        let models = json!([
            { "id": "consus/grok-4.6:itar", "owned_by": "xai", "display_name": "Grok 4.6", "context_window": 500000,
              "max_output_tokens": 500000, "input_modalities": ["text", "image"], "reasoning_efforts": ["low", "medium", "high", "xhigh"],
              "pricing": { "input": 2.64, "output": 7.92, "cache_read": 0.66, "cache_write": 0.0 } },
            { "id": "consus/claude-sonnet-4-5:itar", "owned_by": "anthropic", "display_name": "Claude Sonnet 4.5", "context_window": 200000,
              "max_output_tokens": 64000, "input_modalities": ["text", "image", "pdf"], "reasoning_efforts": ["low", "medium", "high"],
              "pricing": { "input": 3.6, "output": 18.0, "cache_read": 0.36, "cache_write": 4.5 } },
            { "id": "consus/gpt-4.1:itar", "owned_by": "openai", "display_name": "GPT-4.1", "context_window": 300000,
              "max_output_tokens": 32768, "input_modalities": ["text", "image"], "reasoning_efforts": [] },
            { "id": "consus/titan-embed-text-v2:itar", "owned_by": "amazon", "max_output_tokens": null }
        ]);
        let p = provider(&helper(), &models, &Target::default()).unwrap();
        let rows = p["models"].as_array().unwrap();
        let ids: Vec<&str> = rows.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["claude-sonnet-4-5:itar", "gpt-4.1:itar", "grok-4.6:itar"], "Claude first, no embeddings");

        let sonnet = &rows[0];
        assert_eq!(sonnet["name"], "Claude Sonnet 4.5 (ITAR)");
        assert_eq!((sonnet["contextWindow"].as_u64(), sonnet["maxTokens"].as_u64()), (Some(200_000), Some(64_000)));
        assert_eq!(sonnet["input"], json!(["text", "image"]));
        assert_eq!(sonnet["cost"], json!({ "input": 3.6, "output": 18.0, "cacheRead": 0.36, "cacheWrite": 4.5 }));
        assert_eq!(
            sonnet["thinkingLevelMap"],
            json!({ "off": "none", "minimal": "low", "low": "low", "medium": "medium", "high": "high", "xhigh": null })
        );
        assert_eq!(rows[2]["thinkingLevelMap"]["xhigh"], "xhigh");
        assert_eq!(rows[2]["maxTokens"].as_u64(), Some(500_000));

        let gpt = &rows[1];
        assert_eq!(gpt["reasoning"], false);
        assert!(gpt.get("thinkingLevelMap").is_none(), "no efforts, no reasoning levels to offer");
    }
}
