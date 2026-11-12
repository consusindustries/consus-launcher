// Factory: droid, Factory's terminal agent, in an isolated home at
// ~/.factory-consus-gateway, set through FACTORY_HOME_OVERRIDE (droid adds
// .factory under it), so the user's own ~/.factory is never touched.
//
// Docs checked: docs/factory.md, which records what droid 0.245.1 was
// measured to read, through a logging proxy (2026-11-11 and 2026-11-12).
//
// Two files. settings.json, the user level, carries the Consus models, the
// default models, the privacy switches and the sandbox, merged: the launcher
// owns only the customModels entries whose id starts with custom:consus- and
// the keys it writes. consus-org-settings.json is the launcher's own and is
// rewritten whole: the keys droid enforces only from org settings (no web
// search, no image generation, Factory's models refused, custom models only
// at the endpoint) and the default models. The Terminal script names it in
// FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH (undocumented). Without it droid
// offers web search and Factory's own models, so a droid that ignores it is
// refused (check).

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::claude_code::read_object;
use crate::models::{self, Target};
use crate::pi::{maker_rank, write_object};
use crate::policy::policy;

pub const PROFILE_DIR: &str = ".factory-consus-gateway";
const ORG_FILE: &str = "consus-org-settings.json";
const ID_PREFIX: &str = "custom:consus-";
/// How often droid runs the key helper again, as the Consus guide sets it.
const HELPER_TTL_MS: u64 = 60_000;

/// Every default-model key, as a dotted path. Each one defaults to a
/// Factory-hosted model, so each is pinned to a Consus one.
const DEFAULT_KEYS: [&str; 9] = [
    "sessionDefaultSettings.model",
    "sessionDefaultSettings.specModeModel",
    "compactionModel",
    "missionOrchestratorModel",
    "missionModelSettings.workerModel",
    "missionModelSettings.validationWorkerModel",
    "subagentModelSettings.heavyModel",
    "subagentModelSettings.mediumModel",
    "subagentModelSettings.lightModel",
];

/// The home droid is given. Its files are in `.factory` under it.
pub fn profile_dir(home: &Path) -> PathBuf {
    home.join(PROFILE_DIR)
}

fn factory_dir(home: &Path) -> PathBuf {
    profile_dir(home).join(".factory")
}

fn settings_path(home: &Path) -> PathBuf {
    factory_dir(home).join("settings.json")
}

/// The launcher's org-level file.
pub fn org_path(home: &Path) -> PathBuf {
    factory_dir(home).join(ORG_FILE)
}

/// "claude-opus-5-5:itar" -> "custom:consus-claude-opus-5-5". One level per
/// launch, so the level is not needed to tell models apart, and a display
/// name can change without changing the id a user or a script picked. The
/// portal's lock uses the same rule, so its entries merge with these by id.
fn model_id(bare: &str) -> String {
    let base = bare.split_once(':').map_or(bare, |(b, _)| b);
    format!("{ID_PREFIX}{}", base.replace('.', "-"))
}

fn ours(v: &Value) -> bool {
    v.as_str().or_else(|| v.get("id").and_then(Value::as_str)).is_some_and(|id| id.starts_with(ID_PREFIX))
}

fn no_models_message(t: &Target) -> String {
    format!("No {} models for Factory are available to this key.", t.tag())
}

/// Why Factory cannot be set up with this key, if it cannot.
pub fn no_models(models_json: &Value, t: &Target) -> Option<String> {
    models::at_level(models_json, t).is_empty().then(|| no_models_message(t))
}

/// The request format droid uses for a model, and where it sends it: Claude
/// on the Anthropic surface (x-api-key), the GPT models that take the
/// Responses API on OpenAI's, everything else as chat completions. The last
/// two send Bearer, which the gateway accepts.
fn provider(bare: &str, t: &Target) -> (&'static str, String) {
    let base = bare.split_once(':').map_or(bare, |(b, _)| b);
    if base.starts_with("claude-") {
        ("anthropic", t.endpoint.clone())
    } else if crate::chatgpt::RESPONSES_MODELS.contains(&base) {
        ("openai", t.v1())
    } else {
        ("generic-chat-completion-api", t.v1())
    }
}

/// One customModels entry per model this key has at the target level,
/// Claude first, as Pi lists them.
fn custom_models(helper: &Path, models_json: &Value, t: &Target) -> Result<Vec<Value>, String> {
    let tag = t.tag();
    let mut list = models::at_level(models_json, t);
    if list.is_empty() {
        return Err(no_models_message(t));
    }
    list.sort_by(|(_, a), (_, b)| maker_rank(&a.maker).cmp(&maker_rank(&b.maker)).then(a.name.cmp(&b.name)));
    let helper = crate::config::helper_shell(helper);
    Ok(list
        .into_iter()
        .map(|(id, d)| {
            let (provider, base_url) = provider(&id, t);
            json!({
                "id": model_id(&id),
                "model": id,
                "displayName": format!("{} ({tag})", d.name),
                "provider": provider,
                "baseUrl": base_url,
                "maxContextLimit": d.context,
                "maxOutputTokens": d.max_output,
                "apiKeyHelper": helper,
                "apiKeyHelperTtlMs": HELPER_TTL_MS,
            })
        })
        .collect())
}

/// The default-model keys: the newest Opus, else what ChatGPT would start
/// on, else the first model; the medium and light subagents take the first
/// Sonnet and the first Haiku, else that model.
fn defaults(models_json: &Value, t: &Target, ids: &[String]) -> Map<String, Value> {
    let claude: Vec<String> = models::claude_models(models_json, t).into_iter().map(|m| m.id).collect();
    let first = |family: &str| {
        claude.iter().find(|id| id.starts_with(&format!("claude-{family}-"))).map(|id| model_id(id)).filter(|id| ids.contains(id))
    };
    let main = first("opus")
        .or_else(|| crate::chatgpt::select_model(models_json, t).map(|id| model_id(&id)).filter(|id| ids.contains(id)))
        .unwrap_or_else(|| ids[0].clone());
    let mut out = Map::new();
    for path in DEFAULT_KEYS {
        let pick = match path {
            "subagentModelSettings.mediumModel" => first("sonnet"),
            "subagentModelSettings.lightModel" => first("haiku"),
            _ => None,
        };
        set_leaf(&mut out, path, json!(pick.unwrap_or_else(|| main.clone())));
    }
    out
}

fn set_leaf(doc: &mut Map<String, Value>, path: &str, v: Value) {
    match path.split_once('.') {
        None => {
            doc.insert(path.into(), v);
        }
        Some((head, rest)) => {
            let child = doc.entry(head).or_insert_with(|| json!({}));
            if !child.is_object() {
                *child = json!({});
            }
            set_leaf(child.as_object_mut().expect("just made an object"), rest, v);
        }
    }
}

/// Removes a leaf the launcher set (a Consus model id), and an object it
/// leaves empty.
fn remove_ours(doc: &mut Map<String, Value>, path: &str) {
    match path.split_once('.') {
        None => {
            if doc.get(path).is_some_and(ours) {
                doc.remove(path);
            }
        }
        Some((head, rest)) => {
            if let Some(child) = doc.get_mut(head).and_then(Value::as_object_mut) {
                remove_ours(child, rest);
                if child.is_empty() {
                    doc.remove(head);
                }
            }
        }
    }
}

/// `src` into `dst`, object by object, so a key of the user's beside one of
/// the launcher's (in `telemetry`, say) stays.
fn merge(dst: &mut Map<String, Value>, src: &Map<String, Value>) {
    for (k, v) in src {
        match (dst.get_mut(k), v) {
            (Some(Value::Object(d)), Value::Object(s)) => merge(d, s),
            _ => {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
}

pub fn write_config(home: &Path, helper: &Path, models_json: &Value, t: &Target) -> Result<(), String> {
    let mine = custom_models(helper, models_json, t)?;
    let ids: Vec<String> = mine.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect();
    let defaults = defaults(models_json, t, &ids);
    fs::create_dir_all(factory_dir(home)).map_err(|e| e.to_string())?;

    // The user-level file is read before either is written, so a bad one
    // changes nothing; the org-level file is the launcher's alone.
    let spath = settings_path(home);
    let mut doc = read_object(&spath)?;
    let mut list = match doc.remove("customModels") {
        None => Vec::new(),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(format!("{}: 'customModels' is not a list", spath.display())),
    };
    list.retain(|m| !ours(m));
    list.extend(mine);
    doc.insert("customModels".into(), Value::Array(list));
    let p = policy();
    merge(&mut doc, &p.factory_static());
    merge(&mut doc, &p.factory_variant(t.web_access));
    merge(&mut doc, &defaults);

    // The sandbox goes in the org-level file too, so droid's own settings
    // cannot turn it off for the session.
    let mut org = p.factory_org_keys(t);
    merge(&mut org, &p.factory_variant(t.web_access));
    merge(&mut org, &defaults);
    write_object(&org_path(home), org)?;
    write_object(&spath, doc)
}

/// Removes the launcher's models, the defaults that point at them, and the
/// org-level file. The privacy switches stay, and so does the profile with
/// its sessions. Without a profile, nothing to do.
pub fn remove_config(home: &Path) -> Result<(), String> {
    let spath = settings_path(home);
    if spath.exists() {
        let mut doc = read_object(&spath)?;
        let before = doc.clone();
        if let Some(list) = doc.get_mut("customModels").and_then(Value::as_array_mut) {
            list.retain(|m| !ours(m));
        }
        for path in DEFAULT_KEYS {
            remove_ours(&mut doc, path);
        }
        if doc != before {
            write_object(&spath, doc)?;
        }
    }
    match fs::remove_file(org_path(home)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", org_path(home).display())),
        _ => Ok(()),
    }
}

/// What `droid exec --list-tools --output-format json` says about the
/// org-level file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Check {
    /// A tool list without the tools the org-level file removes.
    Pass,
    /// Those tools are listed: droid is not reading the file.
    Refuse,
    /// No answer that can be read.
    Unknown,
}

/// The tools the org-level file removes, as their names read with case and
/// punctuation dropped, so a rename like web_search or WebSearchV2 still
/// counts. Measured with droid 0.245.1: without the file it lists WebSearch
/// (id web_search) and GenerateImage (id generate-image); with it, neither.
const ORG_REMOVED: [&str; 3] = ["websearch", "generateimage", "imagegeneration"];

fn plain(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

pub fn check(output: Option<&str>) -> Check {
    let Some(list) = output.and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok()) else {
        return Check::Unknown;
    };
    let names = |t: &Value| -> Vec<String> {
        ["llmId", "id", "displayName"].iter().filter_map(|k| t.get(*k).and_then(Value::as_str)).map(plain).collect()
    };
    let all: Vec<String> = list.iter().flat_map(names).collect();
    if all.iter().any(|n| ORG_REMOVED.iter().any(|r| n.contains(r))) {
        Check::Refuse
    } else if all.iter().any(|n| n == "read") {
        Check::Pass
    } else {
        Check::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: &str = r#"[
        { "id": "consus/claude-opus-5:itar", "owned_by": "anthropic", "display_name": "Claude Opus 5" },
        { "id": "consus/claude-opus-5-5:itar", "owned_by": "anthropic", "display_name": "Claude Opus 5.5", "context_window": 1000000, "max_output_tokens": 128000 },
        { "id": "consus/claude-sonnet-5-5:itar", "owned_by": "anthropic", "display_name": "Claude Sonnet 5.5" },
        { "id": "consus/claude-haiku-5-5:itar", "owned_by": "anthropic", "display_name": "Claude Haiku 5.5" },
        { "id": "consus/gpt-5.4:itar", "owned_by": "openai", "display_name": "GPT-5.4" },
        { "id": "consus/grok-4.6:itar", "owned_by": "xai", "display_name": "Grok 4.6" },
        { "id": "consus/gpt-5.4:il5+itar", "owned_by": "openai" },
        { "id": "consus/titan-embed-text-v2:itar", "owned_by": "amazon", "max_output_tokens": null }
    ]"#;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consus-launcher-factory-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn helper() -> PathBuf {
        PathBuf::from("/Users/x/Library/Application Support/io.consus.launcher/factory-key-helper")
    }

    fn read(p: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    }

    fn models() -> Value {
        serde_json::from_str(MODELS).unwrap()
    }

    #[test]
    fn writes_every_model_at_the_level_in_the_measured_shape() {
        let home = temp_home("write");
        write_config(&home, &helper(), &models(), &Target::default()).unwrap();
        let s = read(&settings_path(&home));
        let list = s["customModels"].as_array().unwrap();
        let ids: Vec<&str> = list.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            [
                "custom:consus-claude-haiku-5-5",
                "custom:consus-claude-opus-5",
                "custom:consus-claude-opus-5-5",
                "custom:consus-claude-sonnet-5-5",
                "custom:consus-gpt-5-4",
                "custom:consus-grok-4-6"
            ],
            "Claude first, the level stripped, dots as dashes, no embeddings"
        );
        let opus = &list[2];
        assert_eq!(opus["model"], "claude-opus-5-5:itar");
        assert_eq!(opus["displayName"], "Claude Opus 5.5 (ITAR)");
        assert_eq!((opus["provider"].as_str(), opus["baseUrl"].as_str()), (Some("anthropic"), Some("https://api.consus.io")));
        assert_eq!((opus["maxContextLimit"].as_u64(), opus["maxOutputTokens"].as_u64()), (Some(1_000_000), Some(128_000)));
        assert_eq!(opus["apiKeyHelper"], "\"/Users/x/Library/Application Support/io.consus.launcher/factory-key-helper\"");
        assert_eq!(opus["apiKeyHelperTtlMs"], 60_000);
        assert_eq!((list[4]["provider"].as_str(), list[4]["baseUrl"].as_str()), (Some("openai"), Some("https://api.consus.io/v1")));
        assert_eq!(list[5]["provider"], "generic-chat-completion-api");
        assert_eq!(list[5]["baseUrl"], "https://api.consus.io/v1");

        assert_eq!(s["cloudSessionSync"], false);
        assert_eq!(s["telemetry"]["enabled"], false);
        assert_eq!(s["sessionEmbeddingChoice"]["provider"], "off");
        assert_eq!(s["sandbox"], json!({ "enabled": true, "mode": "per-command", "network": { "allowedDomains": [] } }));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_org_file_is_the_required_keys_and_the_defaults() {
        let home = temp_home("org");
        let t = Target { endpoint: "https://ai-proxy.acme.example".into(), web_access: true, ..Target::default() };
        write_config(&home, &helper(), &models(), &t).unwrap();
        let org = read(&org_path(&home));
        assert_eq!(org["modelPolicy"]["allowedBaseUrls"], json!(["https://ai-proxy.acme.example", "https://ai-proxy.acme.example/v1"]));
        assert_eq!(org["sessionDefaultSettings"]["model"], "custom:consus-claude-opus-5-5");
        assert_eq!(org["sandbox"], json!({ "enabled": false }), "the sandbox too, for this web-access setting");
        let mut want = crate::policy::leaf_paths(&Value::Object(policy().factory_org_keys(&t)));
        want.extend(crate::policy::leaf_paths(&Value::Object(policy().factory_variant(t.web_access))));
        want.extend(DEFAULT_KEYS.map(String::from));
        want.sort();
        assert_eq!(crate::policy::leaf_paths(&org), want, "nothing else, no customModels");
        let s = read(&settings_path(&home));
        assert_eq!(s["sandbox"], json!({ "enabled": false }), "web access on");
        assert_eq!(s["customModels"][0]["baseUrl"], "https://ai-proxy.acme.example");
        let off = Target { web_access: false, ..t.clone() };
        write_config(&home, &helper(), &models(), &off).unwrap();
        assert_eq!(read(&org_path(&home))["sandbox"]["mode"], "per-command", "web access off: enforced from the org file");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn defaults_pin_every_key_to_a_consus_model() {
        let ids = |m: &Value| -> Vec<String> {
            custom_models(&helper(), m, &Target::default()).unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect()
        };
        let get = |d: &Map<String, Value>, path: &str| {
            path.split('.').fold(Value::Object(d.clone()), |v, k| v[k].clone()).as_str().unwrap().to_string()
        };
        let full = models();
        let d = defaults(&full, &Target::default(), &ids(&full));
        assert_eq!(get(&d, "sessionDefaultSettings.model"), "custom:consus-claude-opus-5-5", "the newest Opus");
        assert_eq!(get(&d, "compactionModel"), "custom:consus-claude-opus-5-5");
        assert_eq!(get(&d, "subagentModelSettings.mediumModel"), "custom:consus-claude-sonnet-5-5");
        assert_eq!(get(&d, "subagentModelSettings.lightModel"), "custom:consus-claude-haiku-5-5");

        let opus_only = json!([{ "id": "consus/claude-opus-5-5:itar" }]);
        let d = defaults(&opus_only, &Target::default(), &ids(&opus_only));
        assert!(DEFAULT_KEYS.iter().all(|k| get(&d, k) == "custom:consus-claude-opus-5-5"), "one model fills every key");

        let gpt = json!([{ "id": "consus/gpt-oss-120b:itar" }, { "id": "consus/gpt-5.4:itar" }]);
        let d = defaults(&gpt, &Target::default(), &ids(&gpt));
        assert_eq!(get(&d, "sessionDefaultSettings.model"), "custom:consus-gpt-5-4", "without Claude, ChatGPT's model");
        assert_eq!(get(&d, "subagentModelSettings.lightModel"), "custom:consus-gpt-5-4");
    }

    #[test]
    fn the_policy_names_the_keys_that_take_the_main_model() {
        let mut named: Vec<&str> = policy().tool("factory").slot_keys.iter().filter(|(_, s)| *s == "model").map(|(k, _)| k.as_str()).collect();
        let mut want: Vec<&str> = DEFAULT_KEYS.into_iter().filter(|k| !k.ends_with("mediumModel") && !k.ends_with("lightModel")).collect();
        named.sort();
        want.sort();
        assert_eq!(named, want);
    }

    #[test]
    fn keeps_the_users_own_models_and_keys_and_removes_only_the_launchers() {
        let home = temp_home("merge");
        fs::create_dir_all(factory_dir(&home)).unwrap();
        let theirs = json!({ "id": "custom:local-llama", "model": "llama", "baseUrl": "http://localhost:11434/v1" });
        fs::write(
            settings_path(&home),
            json!({
                "customModels": [theirs, { "id": "custom:consus-old-model" }],
                "telemetry": { "enabled": true, "note": "mine" },
                "sessionDefaultSettings": { "autonomyMode": "auto-low" },
                "theme": "dark"
            })
            .to_string(),
        )
        .unwrap();
        write_config(&home, &helper(), &models(), &Target::default()).unwrap();
        let s = read(&settings_path(&home));
        let ids: Vec<&str> = s["customModels"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids[0], "custom:local-llama");
        assert!(!ids.contains(&"custom:consus-old-model"), "a model the key lost is gone");
        assert_eq!(s["telemetry"], json!({ "enabled": false, "note": "mine" }));
        assert_eq!(s["sessionDefaultSettings"]["autonomyMode"], "auto-low");
        assert_eq!(s["theme"], "dark");

        remove_config(&home).unwrap();
        let s = read(&settings_path(&home));
        assert_eq!(s["customModels"], json!([theirs]));
        assert_eq!(s["sessionDefaultSettings"], json!({ "autonomyMode": "auto-low" }));
        assert!(s.get("subagentModelSettings").is_none() && s.get("compactionModel").is_none());
        assert_eq!(s["telemetry"]["enabled"], false, "the privacy switches stay");
        assert!(!org_path(&home).exists());
        assert!(profile_dir(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn without_a_profile_removing_creates_nothing() {
        let home = temp_home("none");
        remove_config(&home).unwrap();
        assert!(!profile_dir(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unreadable_file_changes_nothing() {
        let home = temp_home("unreadable");
        fs::create_dir_all(factory_dir(&home)).unwrap();
        let bad: &[u8] = b"{ \"theme\": \"dark\xff\" }";
        fs::write(settings_path(&home), bad).unwrap();
        assert!(write_config(&home, &helper(), &models(), &Target::default()).is_err());
        assert_eq!(fs::read(settings_path(&home)).unwrap(), bad);
        assert!(!org_path(&home).exists(), "the org-level file must not be written either");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn no_models_at_the_level_is_an_error() {
        let home = temp_home("empty");
        let err = write_config(&home, &helper(), &json!([{ "id": "consus/gemini-3-flash:il4" }]), &Target::default()).unwrap_err();
        assert_eq!(err, "No ITAR models for Factory are available to this key.");
        assert_eq!(no_models(&json!([]), &Target::default()).as_deref(), Some(err.as_str()));
        let _ = fs::remove_dir_all(&home);
    }

    /// droid 0.245.1's `exec --list-tools --output-format json` with the
    /// launcher's org-level file (2026-11-12), trimmed to the fields read.
    const LISTED: &str = r#"[
        { "id": "read-cli", "llmId": "Read" }, { "id": "ls-cli", "llmId": "LS" },
        { "id": "execute-cli", "llmId": "Execute" }, { "id": "fetch_url", "llmId": "FetchUrl" },
        { "id": "automation-create-cli", "llmId": "CreateAutomation" }
    ]"#;

    #[test]
    fn the_version_check_refuses_only_a_listed_web_search() {
        assert_eq!(check(Some(LISTED)), Check::Pass);
        let without_org = LISTED.replace("]", r#", { "id": "web_search", "llmId": "WebSearch" }]"#);
        assert_eq!(check(Some(&without_org)), Check::Refuse);
        let image = LISTED.replace("]", r#", { "id": "generate-image", "llmId": "GenerateImage" }]"#);
        assert_eq!(check(Some(&image)), Check::Refuse);
        for renamed in [r#"{ "llmId": "WebSearchV2" }"#, r#"{ "id": "web-search-cli", "llmId": "Search" }"#, r#"{ "llmId": "X", "displayName": "Web Search" }"#] {
            assert_eq!(check(Some(&LISTED.replace("]", &format!(", {renamed}]")))), Check::Refuse, "{renamed}");
        }
        for unknown in [None, Some(""), Some("not json"), Some("[]"), Some(r#"{ "error": "sign in" }"#)] {
            assert_eq!(check(unknown), Check::Unknown, "{unknown:?}");
        }
    }
}
