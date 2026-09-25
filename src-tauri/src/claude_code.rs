// Claude Code: an isolated profile at ~/.claude-consus-gateway, the same
// CLAUDE_CONFIG_DIR paradigm the Consus guide gives customers, so the user's
// own ~/.claude is never touched.
//
// Docs checked: the Consus integration guide for Claude Code and the Consus
//   portal's Claude Code template (2026-09-22)
// Verified on a real machine: macOS, Claude Code 2.1.281, 2026-09-24. /status
//   showed auth via apiKeyHelper, base URL api.consus.io, User settings as the
//   only source, cwd ~/Consus; a request was answered.
//
// settings.json is merged key by key: the launcher owns apiKeyHelper, model,
// availableModels, and the gateway keys under env; everything else, including
// what Claude Code itself writes there, stays. Sign out removes only those.
// The profile directory is never deleted: the user's history lives in it.

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::models::{self, GATEWAY, REGIME_TAG};

pub const PROFILE_DIR: &str = ".claude-consus-gateway";
/// Where a session starts: empty, visible, and nothing of the user's in scope.
pub const WORK_DIR: &str = "Consus";

// Claude Code's background model must support structured outputs, which the
// gateway does not offer on Sonnet 5 (see the guide). Haiku when the regime
// has one, else these, else anything but Sonnet 5.
const BACKGROUND_PREFERENCE: [&str; 3] = ["claude-haiku-4-5", "claude-sonnet-4-5", "claude-sonnet-4-6"];

const OWNED_TOP: [&str; 3] = ["apiKeyHelper", "model", "availableModels"];
const OWNED_ENV: [&str; 5] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
];

pub fn profile_dir(home: &Path) -> PathBuf {
    home.join(PROFILE_DIR)
}

pub fn work_dir(home: &Path) -> PathBuf {
    home.join(WORK_DIR)
}

fn settings_path(home: &Path) -> PathBuf {
    profile_dir(home).join("settings.json")
}

/// A missing or empty file is an empty object. Any other read or parse
/// failure is an error, never an empty object that would then overwrite
/// the user's settings.
pub fn read_object(path: &Path) -> Result<Map<String, Value>, String> {
    let s = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if s.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&s) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(format!("{}: not a JSON object", path.display())),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The launcher's part of settings.json for these models.
fn owned(helper: &Path, list: &[models::ClaudeModel]) -> Option<(Map<String, Value>, Map<String, Value>)> {
    let first = |family: &str| list.iter().find(|m| m.family == family);
    let main = first("opus").or_else(|| first("sonnet")).or_else(|| list.first())?;
    let opus = first("opus").unwrap_or(main);
    let sonnet = first("sonnet").unwrap_or(main);
    let background = BACKGROUND_PREFERENCE
        .iter()
        .find_map(|b| list.iter().find(|m| m.base == *b))
        .or_else(|| list.iter().find(|m| m.base != "claude-sonnet-5"))
        .unwrap_or(main);

    let mut top = Map::new();
    // apiKeyHelper runs through the shell; the path can contain a space.
    top.insert("apiKeyHelper".into(), json!(format!("\"{}\"", helper.display())));
    top.insert("model".into(), json!(main.id));
    top.insert("availableModels".into(), json!(list.iter().map(|m| &m.id).collect::<Vec<_>>()));

    let mut env = Map::new();
    env.insert("ANTHROPIC_BASE_URL".into(), json!(GATEWAY));
    env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), json!(opus.id));
    env.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), json!(sonnet.id));
    env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), json!(background.id));
    // Claude Code does not recognize model:level ids and would assume 200K.
    if main.version.first().copied().unwrap_or(0) >= 5 {
        env.insert("CLAUDE_CODE_MAX_CONTEXT_TOKENS".into(), json!("1000000"));
    }
    Some((top, env))
}

pub fn write_settings(home: &Path, helper: &Path, models_json: &Value) -> Result<(), String> {
    let list = models::claude_models(models_json);
    let (top, env) =
        owned(helper, &list).ok_or_else(|| format!("No Claude {REGIME_TAG} models are available to this key."))?;

    let path = settings_path(home);
    fs::create_dir_all(profile_dir(home)).map_err(|e| e.to_string())?;
    let mut doc = read_object(&path)?;
    for k in OWNED_TOP {
        doc.remove(k);
    }
    doc.extend(top);
    let env_obj = doc
        .entry("env")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| format!("{}: 'env' is not an object", path.display()))?;
    for k in OWNED_ENV {
        env_obj.remove(k);
    }
    env_obj.extend(env);

    let text = serde_json::to_string_pretty(&Value::Object(doc)).map_err(|e| e.to_string())?;
    fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// Removes the launcher's keys and leaves everything else, including the
/// profile directory. A settings file with nothing left in it is deleted.
pub fn remove_settings(home: &Path) -> Result<(), String> {
    let path = settings_path(home);
    if !path.exists() {
        return Ok(());
    }
    let mut doc = read_object(&path)?;
    for k in OWNED_TOP {
        doc.remove(k);
    }
    if let Some(env) = doc.get_mut("env").and_then(Value::as_object_mut) {
        for k in OWNED_ENV {
            env.remove(k);
        }
        if env.is_empty() {
            doc.remove("env");
        }
    }
    if doc.is_empty() {
        return fs::remove_file(&path).map_err(|e| e.to_string());
    }
    let text = serde_json::to_string_pretty(&Value::Object(doc)).map_err(|e| e.to_string())?;
    fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: &str = r#"[
        { "id": "consus/claude-opus-5-5:itar" },
        { "id": "consus/claude-opus-5:itar" },
        { "id": "consus/claude-sonnet-5:itar" },
        { "id": "consus/claude-sonnet-4-5:itar" },
        { "id": "consus/claude-haiku-4-5:fedramp-high" },
        { "id": "consus/gpt-5.6-terra:itar" }
    ]"#;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consus-launcher-cc-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn helper() -> PathBuf {
        PathBuf::from("/Users/x/Library/Application Support/io.consus.launcher/claude-code-key-helper")
    }

    #[test]
    fn merge_keeps_user_keys_and_picks_the_right_slots() {
        let home = temp_home("merge");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        fs::write(
            settings_path(&home),
            r#"{ "theme": "dark", "effortLevel": "xhigh", "model": "opus[1m]", "env": { "FOO": "1", "ANTHROPIC_BASE_URL": "https://old" } }"#,
        )
        .unwrap();
        write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap()).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();

        assert_eq!(doc["theme"], "dark");
        assert_eq!(doc["effortLevel"], "xhigh");
        assert_eq!(doc["env"]["FOO"], "1");
        assert_eq!(doc["model"], "claude-opus-5-5:itar");
        assert_eq!(doc["env"]["ANTHROPIC_BASE_URL"], "https://api.consus.io");
        assert_eq!(doc["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "claude-opus-5-5:itar");
        assert_eq!(doc["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "claude-sonnet-5:itar");
        assert_eq!(doc["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "claude-sonnet-4-5:itar", "never Sonnet 5 in the background slot");
        assert_eq!(doc["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "1000000");
        assert_eq!(doc["apiKeyHelper"], "\"/Users/x/Library/Application Support/io.consus.launcher/claude-code-key-helper\"");
        let ids: Vec<&str> = doc["availableModels"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(ids, ["claude-opus-5-5:itar", "claude-opus-5:itar", "claude-sonnet-5:itar", "claude-sonnet-4-5:itar"]);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn remove_strips_only_ours_and_keeps_the_directory() {
        let home = temp_home("remove");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        fs::write(settings_path(&home), r#"{ "theme": "dark", "env": { "FOO": "1" } }"#).unwrap();
        write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap()).unwrap();
        remove_settings(&home).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(doc, json!({ "theme": "dark", "env": { "FOO": "1" } }));
        assert!(profile_dir(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn fresh_profile_ends_with_no_settings_file() {
        let home = temp_home("fresh");
        write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap()).unwrap();
        remove_settings(&home).unwrap();
        assert!(!settings_path(&home).exists());
        assert!(profile_dir(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unreadable_settings_file_is_an_error_not_a_wipe() {
        let home = temp_home("unreadable");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        let bad: &[u8] = b"{ \"theme\": \"dark\xff\" }";
        fs::write(settings_path(&home), bad).unwrap();
        assert!(write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap()).is_err());
        assert_eq!(fs::read(settings_path(&home)).unwrap(), bad, "file must be left untouched");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn no_claude_models_is_an_error() {
        let home = temp_home("empty");
        let err = write_settings(&home, &helper(), &json!([{ "id": "consus/gpt-5.4:itar" }])).unwrap_err();
        assert!(err.contains("No Claude"));
        let _ = fs::remove_dir_all(&home);
    }
}
