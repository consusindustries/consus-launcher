// VS Code: an instance of its own, started with --user-data-dir and
// --extensions-dir under ~/.vscode-consus-gateway, so the user's own VS Code
// folders are never read or written.
//
// Docs checked: docs/vscode.md, which records what VS Code 1.147.0 was seen
// to do on macOS and Windows (2026-11-18), and the setting and model schemas
// in that version's own bundle (app/extensions/copilot/package.json and the
// workbench, 2026-11-19).
//
// Three files under data/User. chatLanguageModels.json, an array of provider
// groups: the launcher owns the one named "Consus Gateway" and keeps the
// group's apiKey once it is there, because "Update API Key" in VS Code
// replaces it with a reference to the key VS Code stored. settings.json,
// merged at the top level only: the launcher owns the names in OWNED and the
// rest are the user's. globalStorage/state.vscdb, only when it does not exist
// yet: without the one row it carries, a new instance hides chat behind a
// GitHub sign-in and lists no Consus model.
//
// The key is never written: the person pastes it into VS Code, which keeps it.

use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::claude_code::read_object;
use crate::models::{self, Target};
use crate::pi::{by_maker, write_object};
use crate::policy::policy;

pub const PROFILE_DIR: &str = ".vscode-consus-gateway";
pub const GROUP: &str = "Consus Gateway";
const VENDOR: &str = "customendpoint";
/// The group's apiKey before the person sets the key. VS Code resolves it
/// from its secret store and sends an empty key while it is unset.
const KEY_PLACEHOLDER: &str = "${input:consusApiKey}";
/// The one-row state database: builtinChatExtensionEnablementMigration =
/// true, made with sqlite3 (docs/vscode.md).
const STATE_DB: &[u8] = include_bytes!("../assets/vscode-state.vscdb");

/// Settings the launcher computes from the model list, beside the
/// document's fixed keys.
const MODEL_KEYS: [&str; 3] = ["chat.defaultModel", "chat.utilityModel", "chat.utilitySmallModel"];

pub fn profile_dir(home: &Path) -> PathBuf {
    home.join(PROFILE_DIR)
}

/// What VS Code is given as --user-data-dir.
pub fn data_dir(home: &Path) -> PathBuf {
    profile_dir(home).join("data")
}

/// What VS Code is given as --extensions-dir.
pub fn extensions_dir(home: &Path) -> PathBuf {
    profile_dir(home).join("extensions")
}

fn user_dir(home: &Path) -> PathBuf {
    data_dir(home).join("User")
}

fn models_path(home: &Path) -> PathBuf {
    user_dir(home).join("chatLanguageModels.json")
}

fn settings_path(home: &Path) -> PathBuf {
    user_dir(home).join("settings.json")
}

fn state_path(home: &Path) -> PathBuf {
    user_dir(home).join("globalStorage").join("state.vscdb")
}

/// Every settings name the launcher may write, whichever web-access variant
/// is on, so a flip or a sign out removes the other variant's names too.
fn owned() -> Vec<String> {
    let p = policy();
    let mut names: Vec<String> = p
        .vscode_static()
        .into_iter()
        .chain(p.vscode_variant(false))
        .chain(p.vscode_variant(true))
        .map(|(k, _)| k)
        .chain(MODEL_KEYS.iter().map(|k| k.to_string()))
        .collect();
    names.sort();
    names.dedup();
    names
}

fn no_models_message(t: &Target) -> String {
    format!("No {} models for VS Code are available to this key.", t.tag())
}

/// Why VS Code cannot be set up with this key, if it cannot.
pub fn no_models(models_json: &Value, t: &Target) -> Option<String> {
    models::at_level(models_json, t).is_empty().then(|| no_models_message(t))
}

/// One model row, in the Custom Endpoint provider's shape.
fn row(id: &str, d: &models::Detail, t: &Target) -> Value {
    let mut m = json!({
        "id": id,
        "name": format!("{} ({})", d.name, t.tag()),
        "url": t.v1(),
        // VS Code fills ${apiKey} from the key it stored for the group.
        "requestHeaders": { "x-api-key": "${apiKey}" },
        // The gateway reports no tool-calling flag; every chat model it
        // serves takes tools.
        "toolCalling": true,
        "vision": d.image,
        "maxInputTokens": d.context,
        "maxOutputTokens": d.max_output,
    });
    if !d.efforts.is_empty() {
        m["thinking"] = json!(true);
        m["supportsReasoningEffort"] = json!(d.efforts);
        m["reasoningEffortFormat"] = json!("chat-completions");
    }
    m
}

/// The launcher's group, keeping the apiKey VS Code put there.
fn group(list: &[(String, models::Detail)], t: &Target, api_key: Option<Value>) -> Value {
    json!({
        "name": GROUP,
        "vendor": VENDOR,
        "apiType": "chat-completions",
        "apiKey": api_key.unwrap_or_else(|| json!(KEY_PLACEHOLDER)),
        "models": list.iter().map(|(id, d)| row(id, d, t)).collect::<Vec<_>>(),
    })
}

fn is_ours(v: &Value) -> bool {
    v.get("name").and_then(Value::as_str) == Some(GROUP)
}

/// chatLanguageModels.json: missing or empty is an empty list; anything
/// else that is not a JSON array is an error, and nothing is written.
fn read_array(path: &Path) -> Result<Vec<Value>, String> {
    let s = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if s.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<Value>(&s) {
        Ok(Value::Array(a)) => Ok(a),
        Ok(_) => Err(format!("{}: not a JSON array", path.display())),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Like pi::write_object, for an array: a temp file renamed over the target.
fn write_array(path: &Path, list: Vec<Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&Value::Array(list)).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text + "\n").and_then(|_| fs::rename(&tmp, path)).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// The model settings: the org's start model, else the newest Opus, else
/// what ChatGPT would start on, else the first; titles and summaries on the
/// first Sonnet and the first Haiku in the order, else that model. A chat
/// with no Consus model selected asks for a GitHub sign-in, so the default
/// is always set.
fn model_settings(models_json: &Value, t: &Target, list: &[(String, models::Detail)]) -> Map<String, Value> {
    let ids: Vec<&str> = list.iter().map(|(id, _)| id.as_str()).collect();
    let claude = models::claude_models(models_json, t);
    let claude: Vec<&models::ClaudeModel> = models::ordered(claude.iter().collect(), t.models.as_ref());
    let first = |family: &str| claude.iter().find(|m| m.family == family).map(|m| m.id.clone()).filter(|id| ids.contains(&id.as_str()));
    let main = models::start(list, t.models.as_ref())
        .map(|(id, _)| id.clone())
        .or_else(|| first("opus"))
        .or_else(|| crate::chatgpt::select_model(models_json, t).filter(|id| ids.contains(&id.as_str())))
        .unwrap_or_else(|| ids[0].to_string());
    // chat.defaultModel matches a model's id as the session type registers
    // it: in the Copilot type, VS Code's default and the one the launcher
    // keeps, that is "<vendor>/<group>/<id>" (VS Code 1.147, Windows,
    // 2026-11-19). A Local chat does not match it and starts on the first
    // Consus model instead. The utility settings take "<vendor>/<id>".
    let utility = |id: Option<String>| json!(format!("{VENDOR}/{}", id.unwrap_or_else(|| main.clone())));
    let mut out = Map::new();
    out.insert("chat.utilityModel".into(), utility(first("sonnet")));
    out.insert("chat.utilitySmallModel".into(), utility(first("haiku")));
    out.insert("chat.defaultModel".into(), json!(format!("{VENDOR}/{GROUP}/{main}")));
    out
}

pub fn write_config(home: &Path, models_json: &Value, t: &Target) -> Result<(), String> {
    let own = by_maker(models_json, t);
    if own.is_empty() {
        return Err(no_models_message(t));
    }
    let list = models::ordered(own, t.models.as_ref());
    fs::create_dir_all(user_dir(home)).map_err(|e| e.to_string())?;

    // Read both files before writing either, so a bad one changes nothing.
    let mpath = models_path(home);
    let spath = settings_path(home);
    let mut groups = read_array(&mpath)?;
    let mut sdoc = read_object(&spath).map_err(unmergeable)?;

    let kept = groups.iter().find(|g| is_ours(g)).and_then(|g| g.get("apiKey")).filter(|k| k.is_string()).cloned();
    let mine = group(&list, t, kept);
    match groups.iter_mut().find(|g| is_ours(g)) {
        Some(g) => *g = mine,
        None => groups.push(mine),
    }

    for k in owned() {
        sdoc.remove(&k);
    }
    let p = policy();
    sdoc.extend(p.vscode_static());
    sdoc.extend(p.vscode_variant(t.web_access));
    sdoc.extend(model_settings(models_json, t, &list));

    write_array(&mpath, groups)?;
    write_object(&spath, sdoc)
}

/// What the person can act on when settings.json does not parse: VS Code
/// accepts comments and trailing commas there; the launcher merges plain
/// JSON only, and never rewrites a file it cannot read whole.
fn unmergeable(e: String) -> String {
    format!(
        "VS Code's settings in ~/.vscode-consus-gateway could not be read, so the launcher left them alone. Remove any comments or trailing commas from that settings.json (VS Code: Preferences: Open User Settings (JSON)) and try again. ({e})"
    )
}

/// The state database, before VS Code first starts in this folder. Never
/// opened for writing once it exists, so VS Code's own file is not touched.
/// True when it was written now: a new instance.
pub fn write_state(home: &Path) -> Result<bool, String> {
    let path = state_path(home);
    if path.exists() {
        return Ok(false);
    }
    let dir = path.parent().expect("state.vscdb has a folder");
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join("state.vscdb.consus-tmp");
    let written = fs::File::create(&tmp).and_then(|mut f| f.write_all(STATE_DB).and_then(|_| f.sync_all()));
    // A hard link fails if the target appeared meanwhile, so an existing
    // file is never replaced; the temp file goes either way.
    let linked = written.and_then(|_| fs::hard_link(&tmp, &path));
    let _ = fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(true),
        Err(_) if path.exists() => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Removes the launcher's group and the settings it owns, and leaves the
/// rest, including the folder, its chat history and the state file. The
/// key VS Code stored stays in VS Code's own store.
pub fn remove_config(home: &Path) -> Result<(), String> {
    let mpath = models_path(home);
    if mpath.exists() {
        let mut groups = read_array(&mpath)?;
        let before = groups.len();
        groups.retain(|g| !is_ours(g));
        if groups.len() != before {
            write_array(&mpath, groups)?;
        }
    }
    let spath = settings_path(home);
    if spath.exists() {
        let mut doc = read_object(&spath).map_err(unmergeable)?;
        let before = doc.len();
        for k in owned() {
            doc.remove(&k);
        }
        if doc.len() != before {
            write_object(&spath, doc)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: &str = r#"[
        { "id": "consus/claude-opus-5-5:itar", "owned_by": "anthropic", "display_name": "Claude Opus 5.5", "context_window": 1000000,
          "max_output_tokens": 128000, "input_modalities": ["text", "image"], "reasoning_efforts": ["low", "medium", "high"] },
        { "id": "consus/claude-sonnet-5-5:itar", "owned_by": "anthropic", "display_name": "Claude Sonnet 5.5" },
        { "id": "consus/claude-haiku-5-5:itar", "owned_by": "anthropic", "display_name": "Claude Haiku 5.5" },
        { "id": "consus/gpt-4.1:itar", "owned_by": "openai", "display_name": "GPT-4.1", "reasoning_efforts": [] },
        { "id": "consus/gpt-5.4:il5+itar", "owned_by": "openai" },
        { "id": "consus/titan-embed-text-v2:itar", "owned_by": "amazon", "max_output_tokens": null }
    ]"#;

    fn models() -> Value {
        serde_json::from_str(MODELS).unwrap()
    }

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consus-launcher-vscode-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(p: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    }

    fn ours(home: &Path) -> Value {
        read(&models_path(home)).as_array().unwrap().iter().find(|g| is_ours(g)).cloned().unwrap()
    }

    #[test]
    fn writes_the_group_and_the_settings_from_the_keys_models() {
        let home = temp_home("write");
        write_config(&home, &models(), &Target::default()).unwrap();
        let g = ours(&home);
        assert_eq!(g["vendor"], "customendpoint");
        assert_eq!(g["apiKey"], KEY_PLACEHOLDER);
        let ids: Vec<&str> = g["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            ["claude-haiku-5-5:itar", "claude-opus-5-5:itar", "claude-sonnet-5-5:itar", "gpt-4.1:itar"],
            "the level only, no embeddings, Claude first then by name"
        );
        let opus = &g["models"][1];
        assert_eq!(opus["name"], "Claude Opus 5.5 (ITAR)");
        assert_eq!(opus["url"], "https://api.consus.io/v1");
        assert_eq!(opus["requestHeaders"], json!({ "x-api-key": "${apiKey}" }));
        assert_eq!((opus["toolCalling"].as_bool(), opus["vision"].as_bool()), (Some(true), Some(true)));
        assert_eq!((opus["maxInputTokens"].as_u64(), opus["maxOutputTokens"].as_u64()), (Some(1_000_000), Some(128_000)));
        assert_eq!(opus["supportsReasoningEffort"], json!(["low", "medium", "high"]));
        assert_eq!((opus["thinking"].as_bool(), opus["reasoningEffortFormat"].as_str()), (Some(true), Some("chat-completions")));
        let gpt = &g["models"][3];
        assert!(gpt.get("thinking").is_none() && gpt.get("supportsReasoningEffort").is_none() && gpt.get("reasoningEffortFormat").is_none());

        let s = read(&settings_path(&home));
        assert_eq!(s["chat.defaultModel"], "customendpoint/Consus Gateway/claude-opus-5-5:itar");
        assert_eq!(s["chat.utilityModel"], "customendpoint/claude-sonnet-5-5:itar");
        assert_eq!(s["chat.utilitySmallModel"], "customendpoint/claude-haiku-5-5:itar");
        assert_eq!(s["telemetry.telemetryLevel"], "off");
        assert_eq!(s["chat.agent.networkFilter"], true, "web access is off by default");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn keeps_other_groups_the_persons_settings_and_the_stored_key() {
        let home = temp_home("merge");
        fs::create_dir_all(user_dir(&home)).unwrap();
        fs::write(
            models_path(&home),
            r#"[{ "name": "Local", "vendor": "ollama" }, { "name": "Consus Gateway", "vendor": "customendpoint", "apiKey": "${input:chat.lm.secret.5cffaa26}", "models": [] }]"#,
        )
        .unwrap();
        fs::write(settings_path(&home), r#"{ "editor.fontSize": 15, "chat.defaultModel": "old" }"#).unwrap();
        write_config(&home, &models(), &Target::default()).unwrap();
        let all = read(&models_path(&home));
        assert_eq!(all.as_array().unwrap().len(), 2);
        assert_eq!(all[0], json!({ "name": "Local", "vendor": "ollama" }));
        assert_eq!(ours(&home)["apiKey"], "${input:chat.lm.secret.5cffaa26}", "the key VS Code stored survives");
        assert_eq!(ours(&home)["models"].as_array().unwrap().len(), 4);
        let s = read(&settings_path(&home));
        assert_eq!(s["editor.fontSize"], 15);
        assert_eq!(s["chat.defaultModel"], "customendpoint/Consus Gateway/claude-opus-5-5:itar");

        remove_config(&home).unwrap();
        assert_eq!(read(&models_path(&home)), json!([{ "name": "Local", "vendor": "ollama" }]));
        assert_eq!(read(&settings_path(&home)), json!({ "editor.fontSize": 15 }));
        assert!(profile_dir(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_file_that_does_not_parse_changes_nothing() {
        for (which, bad) in [("settings", "{\n  // VS Code allows comments\n  \"editor.fontSize\": 15\n}\n"), ("models", "[ { \"name\": ")] {
            let home = temp_home(&format!("bad-{which}"));
            fs::create_dir_all(user_dir(&home)).unwrap();
            let (bad_path, other) = if which == "settings" {
                (settings_path(&home), models_path(&home))
            } else {
                (models_path(&home), settings_path(&home))
            };
            fs::write(&bad_path, bad).unwrap();
            let err = write_config(&home, &models(), &Target::default()).unwrap_err();
            if which == "settings" {
                assert!(err.contains("Remove any comments or trailing commas"), "{err}");
            }
            assert_eq!(fs::read_to_string(&bad_path).unwrap(), bad);
            assert!(!other.exists(), "{which}: the other file is not written either");
            assert!(remove_config(&home).is_err());
            assert_eq!(fs::read_to_string(&bad_path).unwrap(), bad);
            let _ = fs::remove_dir_all(&home);
        }
    }

    #[test]
    fn flipping_web_access_leaves_no_stale_keys() {
        let home = temp_home("web");
        let on = Target { web_access: true, ..Target::default() };
        write_config(&home, &models(), &Target::default()).unwrap();
        assert_eq!(read(&settings_path(&home))["chat.agent.sandbox.network.allowedDomains"], json!([]));
        write_config(&home, &models(), &on).unwrap();
        let s = read(&settings_path(&home));
        assert_eq!(s["chat.agent.networkFilter"], false);
        assert!(s.get("chat.agent.sandbox.network.allowedDomains").is_none(), "the off variant's key is gone");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_order_and_level_decide_the_models_and_the_default() {
        let home = temp_home("order");
        let t = Target {
            endpoint: "https://ai-proxy.acme.example".into(),
            models: Some(models::order(&["gpt-4.1", "claude-haiku-5-5"], Some("claude-haiku-5-5"))),
            ..Target::default()
        };
        write_config(&home, &models(), &t).unwrap();
        let g = ours(&home);
        let ids: Vec<&str> = g["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids[..2], ["gpt-4.1:itar", "claude-haiku-5-5:itar"]);
        assert_eq!(g["models"][0]["url"], "https://ai-proxy.acme.example/v1");
        assert_eq!(read(&settings_path(&home))["chat.defaultModel"], "customendpoint/Consus Gateway/claude-haiku-5-5:itar");

        let none = json!([{ "id": "consus/gpt-5.4:il5+itar" }]);
        assert_eq!(write_config(&home, &none, &Target::default()).unwrap_err(), "No ITAR models for VS Code are available to this key.");
        assert!(no_models(&none, &Target::default()).is_some() && no_models(&models(), &Target::default()).is_none());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn without_claude_every_model_setting_is_the_start_model() {
        let home = temp_home("no-claude");
        write_config(&home, &json!([{ "id": "consus/gpt-4.1:itar", "owned_by": "openai" }]), &Target::default()).unwrap();
        let s = read(&settings_path(&home));
        assert_eq!(s["chat.defaultModel"], "customendpoint/Consus Gateway/gpt-4.1:itar");
        assert_eq!(s["chat.utilityModel"], "customendpoint/gpt-4.1:itar");
        assert_eq!(s["chat.utilitySmallModel"], "customendpoint/gpt-4.1:itar");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_state_file_is_written_once_and_never_touched_after() {
        assert!(STATE_DB.starts_with(b"SQLite format 3\0"));
        assert!(STATE_DB.windows(39).any(|w| w == b"builtinChatExtensionEnablementMigration"));
        let home = temp_home("state");
        assert!(write_state(&home).unwrap(), "a new instance");
        assert_eq!(fs::read(state_path(&home)).unwrap(), STATE_DB);
        fs::write(state_path(&home), b"VS Code's own").unwrap();
        assert!(!write_state(&home).unwrap());
        assert_eq!(fs::read(state_path(&home)).unwrap(), b"VS Code's own");
        assert_eq!(fs::read_dir(state_path(&home).parent().unwrap()).unwrap().count(), 1, "no temp file left");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn owned_names_are_the_documents_and_the_model_keys() {
        let names = owned();
        for k in MODEL_KEYS.iter().chain(["telemetry.telemetryLevel", "chat.agent.networkFilter", "chat.agent.sandbox.network.allowedDomains"].iter()) {
            assert!(names.iter().any(|n| n == k), "{k}");
        }
        assert!(!names.iter().any(|n| n.contains("apiKey")));
    }
}
