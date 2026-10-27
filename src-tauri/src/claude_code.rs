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
// settings.json is merged key by key. The launcher owns, by name:
// apiKeyHelper, skipWebFetchPreflight, model, availableModels; the gateway
// and traffic keys under env; the WebSearch and WebFetch rules in
// permissions.deny; and the network keys under sandbox (OWNED_SANDBOX).
// Everything else, including what Claude Code itself writes there and a
// user's own sandbox sub-keys such as excludedCommands, stays. Sign out
// removes only the owned keys. The profile directory is never deleted: the
// user's history lives in it.
//
// Not written here: allowedProviders, which refuses every claude.ai and
// Console sign-in. Claude Code reads it from managed settings only (it is
// inert in a user file), so it belongs in the org's managed-settings.json,
// which the portal's deployment page generates.

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::models::{self, Target};

pub const PROFILE_DIR: &str = ".claude-consus-gateway";
/// Where a session starts: empty, visible, and nothing of the user's in scope.
pub const WORK_DIR: &str = "Consus";

// Claude Code's background model must support structured outputs, which the
// gateway does not offer on Sonnet 5 (see the guide). Haiku when the regime
// has one, else these, else anything but Sonnet 5.
const BACKGROUND_PREFERENCE: [&str; 3] = ["claude-haiku-4-5", "claude-sonnet-4-5", "claude-sonnet-4-6"];

const OWNED_TOP: [&str; 4] = ["apiKeyHelper", "skipWebFetchPreflight", "model", "availableModels"];
const OWNED_ENV: [&str; 13] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL",
    "DISABLE_TELEMETRY",
    "DISABLE_ERROR_REPORTING",
    "DISABLE_FEEDBACK_COMMAND",
    "DISABLE_LOGIN_COMMAND",
    "CLAUDE_CODE_DISABLE_LAUNCHER_SIGN_IN",
    "CLAUDE_CODE_DISABLE_WEB_FETCH",
];
/// The permissions.deny rules the launcher owns (the policy document's
/// web-access variant says which of them a launch writes).
const OWNED_DENY: [&str; 2] = ["WebSearch", "WebFetch"];
/// The paths under `sandbox` the launcher owns (the policy document's
/// web-off variant carries them).
const OWNED_SANDBOX: [&[&str]; 7] = [
    &["enabled"],
    &["failIfUnavailable"],
    &["allowUnsandboxedCommands"],
    &["autoAllowBashIfSandboxed"],
    &["filesystem", "disabled"],
    &["network", "allowedDomains"],
    &["network", "strictAllowlist"],
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
fn owned(
    helper: &Path,
    list: &[models::ClaudeModel],
    models_json: &Value,
    t: &Target,
) -> Option<(Map<String, Value>, Map<String, Value>)> {
    let first = |family: &str| list.iter().find(|m| m.family == family);
    let main = first("opus").or_else(|| first("sonnet")).or_else(|| list.first())?;
    let opus = first("opus").unwrap_or(main);
    let sonnet = first("sonnet").unwrap_or(main);
    let background = BACKGROUND_PREFERENCE
        .iter()
        .find_map(|b| list.iter().find(|m| m.base == *b))
        .or_else(|| list.iter().find(|m| m.base != "claude-sonnet-5"))
        .unwrap_or(main);

    // The static keys and env come from the policy document: zero traffic to
    // Anthropic (analytics, error reports, update checks, flag calls, the
    // plugin marketplace download, the WebFetch hostname check, each
    // measured to cover calls the others miss on Claude Code 2.1.300), and
    // no sign-in path (/login, the launcher sign-in) that would route around
    // the gateway. Only the slots are filled here.
    let policy = crate::policy::policy();
    let mut top = policy.claude_code_static();
    // apiKeyHelper runs through the shell; the path can contain a space.
    top.insert("apiKeyHelper".into(), json!(format!("\"{}\"", crate::config::command_path(helper))));
    top.insert("model".into(), json!(main.id));
    top.insert("availableModels".into(), json!(list.iter().map(|m| &m.id).collect::<Vec<_>>()));

    let mut env = policy.claude_code_env_static();
    env.insert("ANTHROPIC_BASE_URL".into(), json!(t.endpoint));
    env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), json!(opus.id));
    env.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), json!(sonnet.id));
    env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), json!(background.id));
    // Web access off is three things, kept together in write_settings: the
    // variant's env removes the WebFetch tool (2.1.285 and later), the deny
    // rule covers earlier versions, and the sandbox closes the shell.
    env.extend(policy.claude_code_variant(t.web_access).0);
    // Claude Code does not recognize model:level ids and would assume 200K,
    // so a larger window is spelled out: the gateway's, or 1M for a 5-series
    // model from a gateway that does not report windows.
    let window = match models::row(models_json, &main.id) {
        Some(r) if r.get("context_window").is_some() => Some(models::detail(r).context),
        _ => (main.version.first().copied().unwrap_or(0) >= 5).then_some(1_000_000),
    };
    if let Some(w) = window.filter(|w| *w > 200_000) {
        env.insert("CLAUDE_CODE_MAX_CONTEXT_TOKENS".into(), json!(w.to_string()));
    }
    Some((top, env))
}

/// Removes the launcher's paths under `sandbox`, pruning objects that are
/// left empty, then writes `ours` into it key by key. The user's other
/// sandbox keys stay.
fn set_sandbox(doc: &mut Map<String, Value>, ours: Option<Value>) -> Result<(), String> {
    fn remove_path(obj: &mut Map<String, Value>, path: &[&str]) {
        match path {
            [] => {}
            [k] => {
                obj.remove(*k);
            }
            [k, rest @ ..] => {
                if let Some(child) = obj.get_mut(*k).and_then(Value::as_object_mut) {
                    remove_path(child, rest);
                    if child.is_empty() {
                        obj.remove(*k);
                    }
                }
            }
        }
    }
    fn merge(dst: &mut Map<String, Value>, src: Map<String, Value>) -> Result<(), String> {
        for (k, v) in src {
            match v {
                Value::Object(sub) => {
                    let child = dst
                        .entry(k.clone())
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .ok_or_else(|| format!("'sandbox.{k}' is not an object"))?;
                    merge(child, sub)?;
                }
                other => {
                    dst.insert(k, other);
                }
            }
        }
        Ok(())
    }
    if let Some(sandbox) = doc.get_mut("sandbox") {
        let obj = sandbox.as_object_mut().ok_or("'sandbox' is not an object")?;
        for path in OWNED_SANDBOX {
            remove_path(obj, path);
        }
        if obj.is_empty() {
            doc.remove("sandbox");
        }
    }
    if let Some(Value::Object(ours)) = ours {
        let obj = doc
            .entry("sandbox")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("'sandbox' is not an object")?;
        merge(obj, ours)?;
    }
    Ok(())
}

pub fn write_settings(home: &Path, helper: &Path, models_json: &Value, t: &Target) -> Result<(), String> {
    let list = models::claude_models(models_json, t);
    let (top, env) =
        owned(helper, &list, models_json, t).ok_or_else(|| format!("No Claude {} models are available to this key.", t.tag()))?;

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

    let (_, deny, sandbox) = crate::policy::policy().claude_code_variant(t.web_access);
    let deny: Vec<&str> = deny.iter().map(String::as_str).collect();
    set_deny(&mut doc, &deny).map_err(|e| format!("{}: {e}", path.display()))?;
    set_sandbox(&mut doc, sandbox).map_err(|e| format!("{}: {e}", path.display()))?;

    let text = serde_json::to_string_pretty(&Value::Object(doc)).map_err(|e| e.to_string())?;
    fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// Replaces the launcher's entries in permissions.deny with `ours`, keeping
/// every other rule. A container left empty goes too; callers only get here
/// when there is something to write or remove, so an empty list that was
/// already there is never touched.
fn set_deny(doc: &mut Map<String, Value>, ours: &[&str]) -> Result<(), String> {
    let perms = doc
        .entry("permissions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("'permissions' is not an object")?;
    let deny = perms
        .entry("deny")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("'permissions.deny' is not a list")?;
    deny.retain(|v| !v.as_str().is_some_and(|r| OWNED_DENY.contains(&r)));
    deny.extend(ours.iter().map(|r| json!(r)));
    if deny.is_empty() {
        perms.remove("deny");
    }
    if perms.is_empty() {
        doc.remove("permissions");
    }
    Ok(())
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
    // Map indexing panics on a missing key; get() does not.
    let has_ours = doc
        .get("permissions")
        .and_then(|p| p.get("deny"))
        .and_then(Value::as_array)
        .is_some_and(|d| d.iter().any(|v| v.as_str().is_some_and(|r| OWNED_DENY.contains(&r))));
    if has_ours {
        set_deny(&mut doc, &[])?;
    }
    if doc.get("sandbox").is_some_and(Value::is_object) {
        set_sandbox(&mut doc, None)?;
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
    fn the_owned_keys_are_exactly_what_the_policy_document_describes() {
        // The document is the source; these consts are the writer's view of it.
        let p = crate::policy::policy();
        let cc = p.tool("claude-code");
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        let top: Vec<String> = cc.settings.keys().chain(cc.slot_keys.keys()).cloned().collect();
        assert_eq!(sorted(top), sorted(OWNED_TOP.iter().map(|s| s.to_string()).collect()));
        let mut env: Vec<String> = cc.env.keys().chain(cc.env_slot_keys.keys()).cloned().collect();
        for on in [false, true] {
            env.extend(p.claude_code_variant(on).0.keys().cloned());
        }
        env.sort();
        env.dedup();
        assert_eq!(env, sorted(OWNED_ENV.iter().map(|s| s.to_string()).collect()));
        let mut deny: Vec<String> = [false, true].into_iter().flat_map(|on| p.claude_code_variant(on).1).collect();
        deny.sort();
        deny.dedup();
        assert_eq!(deny, sorted(OWNED_DENY.iter().map(|s| s.to_string()).collect()));
        let mut sandbox: Vec<String> =
            [false, true].into_iter().filter_map(|on| p.claude_code_variant(on).2).flat_map(|s| crate::policy::leaf_paths(&s)).collect();
        sandbox.sort();
        sandbox.dedup();
        assert_eq!(sandbox, sorted(OWNED_SANDBOX.iter().map(|p| p.join(".")).collect()));
        // And what the writer writes is the document's.
        let (top_doc, _) = owned(Path::new("/h"), &models::claude_models(&serde_json::from_str(MODELS).unwrap(), &Target::default()), &serde_json::from_str(MODELS).unwrap(), &Target::default()).unwrap();
        for (k, v) in &cc.settings {
            assert_eq!(&top_doc[k], v, "{k}");
        }
        let (_, env_doc) = owned(Path::new("/h"), &models::claude_models(&serde_json::from_str(MODELS).unwrap(), &Target::default()), &serde_json::from_str(MODELS).unwrap(), &Target::default()).unwrap();
        for (k, v) in p.claude_code_env_static() {
            assert_eq!(env_doc[&k], v, "{k}");
        }
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
        write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &Target::default()).unwrap();
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
        assert_eq!(doc["skipWebFetchPreflight"], true);
        assert_eq!(doc["env"]["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], "1");
        assert_eq!(doc["env"]["CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL"], "1");
        assert_eq!(doc["env"]["DISABLE_LOGIN_COMMAND"], "1");
        assert_eq!(doc["env"]["CLAUDE_CODE_DISABLE_LAUNCHER_SIGN_IN"], "1");
        assert_eq!(doc["env"]["DISABLE_TELEMETRY"], "1");
        assert_eq!(doc["env"]["DISABLE_ERROR_REPORTING"], "1");
        assert_eq!(doc["env"]["DISABLE_FEEDBACK_COMMAND"], "1");
        assert_eq!(doc["env"]["CLAUDE_CODE_DISABLE_WEB_FETCH"], "1");
        assert_eq!(doc["permissions"]["deny"], json!(["WebSearch", "WebFetch"]));
        assert_eq!(doc["sandbox"]["enabled"], true);
        assert_eq!(doc["sandbox"]["network"]["allowedDomains"], json!([]));
        assert_eq!(doc["sandbox"]["network"]["strictAllowlist"], true);
        assert_eq!(doc["sandbox"]["allowUnsandboxedCommands"], false);
        assert_eq!(doc["sandbox"]["filesystem"]["disabled"], true, "network only");
        assert_eq!(doc["apiKeyHelper"], "\"/Users/x/Library/Application Support/io.consus.launcher/claude-code-key-helper\"");
        let ids: Vec<&str> = doc["availableModels"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(ids, ["claude-opus-5-5:itar", "claude-opus-5:itar", "claude-sonnet-5:itar", "claude-sonnet-4-5:itar"]);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn remove_strips_only_ours_and_keeps_the_directory() {
        let home = temp_home("remove");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        fs::write(
            settings_path(&home),
            r#"{ "theme": "dark", "env": { "FOO": "1" }, "permissions": { "deny": ["Read(./.env)", "WebFetch"], "allow": ["Bash(ls)"] } }"#,
        )
        .unwrap();
        write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &Target::default()).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(doc["permissions"]["deny"], json!(["Read(./.env)", "WebSearch", "WebFetch"]), "the user's rule stays, ours once");
        remove_settings(&home).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(
            doc,
            json!({ "theme": "dark", "env": { "FOO": "1" }, "permissions": { "deny": ["Read(./.env)"], "allow": ["Bash(ls)"] } })
        );
        assert!(profile_dir(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_users_own_sandbox_keys_survive_in_both_states_and_on_sign_out() {
        let home = temp_home("sandbox-theirs");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        let theirs = r#"{ "sandbox": { "excludedCommands": ["docker compose *"], "network": { "allowUnixSockets": ["/tmp/ssh-agent"], "allowedDomains": ["github.com"] } } }"#;
        fs::write(settings_path(&home), theirs).unwrap();
        let models: Value = serde_json::from_str(MODELS).unwrap();

        write_settings(&home, &helper(), &models, &Target::default()).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(doc["sandbox"]["enabled"], true);
        assert_eq!(doc["sandbox"]["network"]["allowedDomains"], json!([]), "web off: the launcher's list, not the user's");
        assert_eq!(doc["sandbox"]["network"]["allowUnixSockets"], json!(["/tmp/ssh-agent"]), "the user's own key stays");
        assert_eq!(doc["sandbox"]["excludedCommands"], json!(["docker compose *"]));

        write_settings(&home, &helper(), &models, &Target { web_access: true, ..Target::default() }).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert!(doc["sandbox"].get("enabled").is_none(), "web on: the launcher's keys go");
        assert!(doc["sandbox"]["network"].get("allowedDomains").is_none());
        assert_eq!(doc["sandbox"]["network"]["allowUnixSockets"], json!(["/tmp/ssh-agent"]));
        assert_eq!(doc["sandbox"]["excludedCommands"], json!(["docker compose *"]));

        remove_settings(&home).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(
            doc,
            json!({ "sandbox": { "excludedCommands": ["docker compose *"], "network": { "allowUnixSockets": ["/tmp/ssh-agent"] } } })
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn signing_out_twice_or_from_a_file_without_permissions_is_fine() {
        let home = temp_home("remove-twice");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        fs::write(settings_path(&home), r#"{ "theme": "dark", "apiKeyHelper": "x" }"#).unwrap();
        remove_settings(&home).unwrap();
        remove_settings(&home).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(doc, json!({ "theme": "dark" }));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_deny_list_without_our_rules_is_left_alone_on_sign_out() {
        let home = temp_home("deny-theirs");
        fs::create_dir_all(profile_dir(&home)).unwrap();
        let theirs = r#"{ "permissions": { "deny": [] }, "apiKeyHelper": "x" }"#;
        fs::write(settings_path(&home), theirs).unwrap();
        remove_settings(&home).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(doc, json!({ "permissions": { "deny": [] } }), "an empty list the user placed stays");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn web_access_on_keeps_fetch_and_still_denies_search() {
        let home = temp_home("web-on");
        let t = Target { web_access: true, ..Target::default() };
        write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &t).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert!(doc["env"].get("CLAUDE_CODE_DISABLE_WEB_FETCH").is_none());
        assert!(doc.get("sandbox").is_none(), "the shell reaches the network when the org allows the web");
        assert_eq!(doc["permissions"]["deny"], json!(["WebSearch"]));
        // Turning it back off on the next launch replaces, never duplicates.
        write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &Target::default()).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(doc["permissions"]["deny"], json!(["WebSearch", "WebFetch"]));
        assert_eq!(doc["sandbox"]["enabled"], true);
        remove_settings(&home).unwrap();
        assert!(!settings_path(&home).exists(), "sandbox is the launcher's key and goes with the rest");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn fresh_profile_ends_with_no_settings_file() {
        let home = temp_home("fresh");
        write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &Target::default()).unwrap();
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
        assert!(write_settings(&home, &helper(), &serde_json::from_str(MODELS).unwrap(), &Target::default()).is_err());
        assert_eq!(fs::read(settings_path(&home)).unwrap(), bad, "file must be left untouched");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_context_limit_follows_the_gateways_window() {
        let limit = |models: Value| {
            let list = models::claude_models(&models, &Target::default());
            owned(Path::new("/h"), &list, &models, &Target::default()).unwrap().1.get("CLAUDE_CODE_MAX_CONTEXT_TOKENS").cloned()
        };
        assert_eq!(limit(json!([{ "id": "consus/claude-opus-5-5:itar", "context_window": 1000000 }])), Some(json!("1000000")));
        assert_eq!(limit(json!([{ "id": "consus/claude-opus-5-5:itar", "context_window": 500000 }])), Some(json!("500000")));
        assert_eq!(limit(json!([{ "id": "consus/claude-sonnet-4-5:itar", "context_window": 200000 }])), None, "Claude Code's own default");
        assert_eq!(limit(json!([{ "id": "consus/claude-opus-5:itar" }])), Some(json!("1000000")), "no window reported: as before");
    }

    #[test]
    fn no_claude_models_is_an_error() {
        let home = temp_home("empty");
        let err = write_settings(&home, &helper(), &json!([{ "id": "consus/gpt-5.4:itar" }]), &Target::default()).unwrap_err();
        assert!(err.contains("No Claude"));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn follows_the_target_level_and_endpoint() {
        let home = temp_home("target");
        let t = Target { endpoint: "https://ai-proxy.acme.example".into(), level: "fedramp-high".into(), ..Target::default() };
        let models = json!([{ "id": "consus/claude-opus-5-5:fedramp-high" }, { "id": "consus/claude-sonnet-4-5:itar" }]);
        write_settings(&home, &helper(), &models, &t).unwrap();
        let doc: Value = serde_json::from_str(&fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(doc["env"]["ANTHROPIC_BASE_URL"], "https://ai-proxy.acme.example");
        assert_eq!(doc["model"], "claude-opus-5-5:fedramp-high");
        let _ = fs::remove_dir_all(&home);
    }
}
