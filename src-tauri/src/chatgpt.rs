// ChatGPT desktop app (ChatGPT Work and Codex modes): ~/.codex/config.toml,
// one per-user file shared with Codex CLI, which the app also writes to.
//
// Docs checked: gov_open_router/docs/integrations/chatgpt-desktop.md and
//   consus-key-portal lib/deploy/templates/codex.ts (2026-09-22)
// Verified on a real machine: macOS, ChatGPT 26.915, 2026-09-22. Launched from
//   the launcher with the key in the environment; a request was answered and the
//   app's footer read "Consus Gateway".
//
// The launcher spawns the app itself, so the key travels in that process's
// environment (env_http_headers) and is never written to the file. Every key
// and table the app or the user put in the file is preserved; only the
// launcher's own keys and tables are replaced. In TOML, top-level keys must
// sit above the first [table] header; toml_edit keeps them there.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut};

const REGIME: &str = "itar";
const REGIME_TAG: &str = "ITAR";
// Only these GPT models are served over the Responses API the app speaks,
// in preference order. The models endpoint does not expose this.
const RESPONSES_MODELS: [&str; 5] = ["gpt-5.6-terra", "gpt-5.6-sol", "gpt-5.6-luna", "gpt-5.4", "gpt-5.1"];

// Verbatim from the guide, minus the inline key: env_http_headers instead.
const TEMPLATE: &str = r#"model_provider = "consus"
model_reasoning_effort = "medium"
model_reasoning_summary = "auto"
web_search = "disabled"
approval_policy = "on-request"
approvals_reviewer = "user"
sandbox_mode = "workspace-write"

[sandbox_workspace_write]
network_access = false

[model_providers.consus]
name = "Consus Gateway"
base_url = "https://api.consus.io/v1"
wire_api = "responses"
env_http_headers = { "x-api-key" = "CONSUS_API_KEY" }

[shell_environment_policy]
inherit = "all"
exclude = ["AWS_*", "AZURE_*", "GOOGLE_*", "GCP_*", "CONSUS_*", "OPENAI_*", "ANTHROPIC_*", "*_KEY", "*_TOKEN", "*_SECRET", "*PASSWORD*"]

[features]
computer_use = false
browser_use = false
browser_use_external = false
browser_use_full_cdp_access = false
in_app_browser = false
image_generation = false
realtime_conversation = false
in_app_dictation = false
apps = false
remote_plugin = false
plugin_sharing = false
recommended_plugins = false
tool_suggest = false
skill_mcp_dependency_install = false
memories = false

[computer_use]
default_app_access = "deny"

[browser_use]
allow_history_access = false

[browser_use.default_origin_policy]
access = "deny"
uploads = "deny"
downloads = "deny"
full_cdp_access = "deny"

[memories]
use_memories = false
generate_memories = false

[analytics]
enabled = false

[feedback]
enabled = false

[otel]
exporter = "none"
trace_exporter = "none"
metrics_exporter = "none"
log_user_prompt = false
"#;

/// The one model the app starts on: the most preferred Responses-served GPT
/// model this key can use in the regime, as a bare id ("gpt-5.6-terra:itar").
pub fn select_model(models: &Value) -> Option<String> {
    let ids: Vec<&str> = models
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .map(|id| id.strip_prefix("consus/").unwrap_or(id))
        .collect();
    RESPONSES_MODELS.iter().find_map(|base| {
        let want = format!("{base}:{REGIME}");
        ids.iter().find(|id| **id == want).map(|id| id.to_string())
    })
}

fn config_path(home: &Path) -> PathBuf {
    home.join(".codex/config.toml")
}

fn backup_path(home: &Path) -> PathBuf {
    home.join(".codex/config.toml.consus-bak")
}

pub fn write_config(home: &Path, models: &Value) -> Result<(), String> {
    let model = select_model(models)
        .ok_or_else(|| format!("No GPT {REGIME_TAG} models are available to this key."))?;

    let path = config_path(home);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut doc: DocumentMut = existing
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let tmpl: DocumentMut = TEMPLATE.parse().map_err(|e| format!("template: {e}"))?;

    for (k, item) in tmpl.as_table().iter() {
        if k == "model_providers" {
            // Other providers the user defined stay; only ours is replaced.
            doc["model_providers"]["consus"] = item["consus"].clone();
        } else {
            doc[k] = item.clone();
        }
    }
    doc["model"] = value(model);

    let bak = backup_path(home);
    if path.exists() && !bak.exists() {
        fs::copy(&path, &bak).map_err(|e| e.to_string())?;
    }
    fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Puts the file back the way it was before the launcher first touched it.
/// If there was no file then, the launcher's is removed.
pub fn restore(home: &Path) -> Result<(), String> {
    let path = config_path(home);
    let bak = backup_path(home);
    if bak.exists() {
        fs::copy(&bak, &path).map_err(|e| e.to_string())?;
        fs::remove_file(&bak).map_err(|e| e.to_string())?;
    } else if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Shaped like a real file the app and a user have both written to.
    const EXISTING: &str = r#"# Consus Gateway: compliance baseline
model = "gpt-5.6-sol:itar"
model_provider = "consus"
model_reasoning_effort = "medium"
model_catalog_json = "/Users/x/.codex-gateway/consus-models.json"
web_search = "disabled"
notify = ["/Users/x/Client", "turn-ended"]

[sandbox_workspace_write]
network_access = false

[model_providers.consus]
name = "Consus Gateway"
base_url = "https://api.consus.io/v1"
wire_api = "responses"
http_headers = { "x-api-key" = "REDACTED_KEY" }

[model_providers.other]
name = "Something else"

[features]
computer_use = false

[projects."/Users/x/proj"]
trust_level = "trusted"

[desktop]
followUpQueueMode = "steer"
"#;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consus-launcher-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".codex")).unwrap();
        dir
    }

    #[test]
    fn merge_keeps_user_content_and_drops_inline_key() {
        let home = temp_home("merge");
        fs::write(config_path(&home), EXISTING).unwrap();
        let models = json!([
            { "id": "consus/claude-opus-5:itar" },
            { "id": "consus/gpt-5.6-sol:itar" },
            { "id": "consus/gpt-5.6-terra:itar" },
        ]);
        write_config(&home, &models).unwrap();
        let out = fs::read_to_string(config_path(&home)).unwrap();

        assert!(out.contains("model = \"gpt-5.6-terra:itar\""));
        assert!(out.contains("env_http_headers"));
        assert!(!out.contains("REDACTED_KEY"));
        assert!(!out.contains("\nhttp_headers"));
        assert!(out.contains("[model_providers.other]"));
        assert!(out.contains("[projects.\"/Users/x/proj\"]"));
        assert!(out.contains("followUpQueueMode"));
        assert!(out.contains("# Consus Gateway: compliance baseline"));

        let first_table = out.find("\n[").unwrap();
        for key in ["model = ", "model_provider = ", "model_catalog_json", "notify = ["] {
            assert!(out.find(key).unwrap() < first_table, "{key} must sit above the first table");
        }

        assert!(backup_path(&home).exists());
        restore(&home).unwrap();
        assert_eq!(fs::read_to_string(config_path(&home)).unwrap(), EXISTING);
        assert!(!backup_path(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn no_file_before_means_none_after_restore() {
        let home = temp_home("fresh");
        write_config(&home, &json!([{ "id": "consus/gpt-5.4:itar" }])).unwrap();
        assert!(config_path(&home).exists());
        assert!(!backup_path(&home).exists());
        restore(&home).unwrap();
        assert!(!config_path(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn picks_by_preference_and_regime() {
        let m = json!([
            { "id": "consus/gpt-5.1:itar" },
            { "id": "consus/gpt-5.6-sol:fedramp-high" },
            { "id": "consus/gpt-4.1:itar" },
        ]);
        assert_eq!(select_model(&m).as_deref(), Some("gpt-5.1:itar"));
        assert_eq!(select_model(&json!([{ "id": "consus/claude-opus-5:itar" }])), None);
    }
}
