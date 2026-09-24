// ChatGPT desktop app (ChatGPT Work and Codex modes): ~/.codex/config.toml,
// one per-user file shared with Codex CLI, which the app also writes to. The
// template and its provenance live in templates/chatgpt-desktop.toml.
//
// The launcher spawns the app itself, so the key travels in that process's
// environment (env_http_headers) and is never written to the file. The
// template is merged key by key: keys and tables the app or the user put in
// the file stay, and sign out removes only the template's own keys. In TOML,
// top-level keys must sit above the first [table] header; toml_edit keeps
// them there. The app rewrites this file on quit, so callers write it only
// while the app is not running.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, Table, TableLike};

use crate::models::{self, REGIME, REGIME_TAG};

// Only these GPT models are served over the Responses API the app speaks,
// in preference order. The models endpoint does not expose this.
const RESPONSES_MODELS: [&str; 5] = ["gpt-5.6-terra", "gpt-5.6-sol", "gpt-5.6-luna", "gpt-5.4", "gpt-5.1"];

// The guide's config, minus the inline key. Provenance is in the file.
const TEMPLATE: &str = include_str!("../templates/chatgpt-desktop.toml");

/// The one model the app starts on: the most preferred Responses-served GPT
/// model this key can use in the regime, as a bare id ("gpt-5.6-terra:itar").
pub fn select_model(models_json: &Value) -> Option<String> {
    let ids = models::ids(models_json);
    RESPONSES_MODELS.iter().find_map(|base| {
        let want = format!("{base}:{REGIME}");
        ids.iter().find(|id| **id == want).map(|id| id.to_string())
    })
}

fn config_path(home: &Path) -> PathBuf {
    home.join(".codex/config.toml")
}

fn template() -> DocumentMut {
    TEMPLATE.parse().expect("template is valid TOML")
}

/// Replaces a key's value in place so the key keeps its position and the
/// comments around it; a key that is not there yet is appended.
fn set_leaf(dst: &mut dyn TableLike, k: &str, item: Item) {
    match dst.get_mut(k) {
        Some(existing) => {
            let decor = existing.as_value().map(|v| v.decor().clone());
            *existing = item;
            if let (Some(d), Some(v)) = (decor, existing.as_value_mut()) {
                *v.decor_mut() = d;
            }
        }
        None => {
            dst.insert(k, item);
        }
    }
}

/// Copies every key of `src` into `dst`, recursing into tables so keys the
/// app or the user added inside them survive. A key that exists in `dst`
/// as something other than a table where `src` has a table is an error.
fn merge_into(dst: &mut dyn TableLike, src: &Table, path: &str) -> Result<(), String> {
    for (k, item) in src.iter() {
        let here = if path.is_empty() { k.to_string() } else { format!("{path}.{k}") };
        match item.as_table() {
            Some(sub) => {
                if dst.get(k).is_none() {
                    let mut t = Table::new();
                    t.set_implicit(sub.is_implicit());
                    dst.insert(k, Item::Table(t));
                }
                let child = dst
                    .get_mut(k)
                    .and_then(Item::as_table_like_mut)
                    .ok_or_else(|| format!("config.toml: '{here}' is not a table"))?;
                merge_into(child, sub, &here)?;
            }
            None => set_leaf(dst, k, item.clone()),
        }
    }
    Ok(())
}

/// The inverse of merge_into: removes `src`'s keys from `dst`, and a table
/// only once nothing else is left in it.
fn strip_from(dst: &mut dyn TableLike, src: &Table) {
    for (k, item) in src.iter() {
        match item.as_table() {
            Some(sub) => {
                let empty = match dst.get_mut(k).and_then(Item::as_table_like_mut) {
                    Some(child) => {
                        strip_from(child, sub);
                        child.is_empty()
                    }
                    None => false,
                };
                if empty {
                    dst.remove(k);
                }
            }
            None => {
                dst.remove(k);
            }
        }
    }
}

pub fn write_config(home: &Path, models_json: &Value) -> Result<(), String> {
    write_config_at(&config_path(home), models_json, &Table::new())
}

/// The template, then `extra` (keys only this file carries), merged into
/// the config.toml at `path`.
pub fn write_config_at(path: &Path, models_json: &Value, extra: &Table) -> Result<(), String> {
    let model = select_model(models_json)
        .ok_or_else(|| format!("No GPT {REGIME_TAG} models are available to this key."))?;

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let existing = fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = existing
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    merge_into(doc.as_table_mut(), template().as_table(), "")?;
    merge_into(doc.as_table_mut(), extra, "")?;
    set_leaf(doc.as_table_mut(), "model", value(model));
    // A hand-made config carries the key inline; the environment replaces it.
    if let Some(consus) = doc
        .get_mut("model_providers")
        .and_then(Item::as_table_like_mut)
        .and_then(|t| t.get_mut("consus"))
        .and_then(Item::as_table_like_mut)
    {
        consus.remove("http_headers");
    }

    fs::write(path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Removes the launcher's keys and leaves everything else; a comment
/// attached to a removed key goes with it. A file with nothing left in it
/// is deleted.
pub fn remove_config(home: &Path) -> Result<(), String> {
    remove_config_at(&config_path(home), &Table::new())
}

pub fn remove_config_at(path: &Path, extra: &Table) -> Result<(), String> {
    let Ok(existing) = fs::read_to_string(path) else {
        return Ok(());
    };
    let mut doc: DocumentMut = existing
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    strip_from(doc.as_table_mut(), template().as_table());
    strip_from(doc.as_table_mut(), extra);
    doc.as_table_mut().remove("model");
    if doc.as_table().is_empty() {
        return fs::remove_file(path).map_err(|e| e.to_string());
    }
    fs::write(path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))
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
computer_use = true
custom_flag = true

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

    const MODELS: &str = r#"[
        { "id": "consus/claude-opus-5:itar" },
        { "id": "consus/gpt-5.6-sol:itar" },
        { "id": "consus/gpt-5.6-terra:itar" }
    ]"#;

    #[test]
    fn merge_keeps_user_content_and_drops_inline_key() {
        let home = temp_home("merge");
        fs::write(config_path(&home), EXISTING).unwrap();
        write_config(&home, &serde_json::from_str(MODELS).unwrap()).unwrap();
        let out = fs::read_to_string(config_path(&home)).unwrap();

        assert!(out.contains("model = \"gpt-5.6-terra:itar\""));
        assert!(out.contains("env_http_headers"));
        assert!(!out.contains("REDACTED_KEY"));
        assert!(!out.contains("\nhttp_headers"));
        assert!(out.contains("computer_use = false"), "template value wins");
        assert!(out.contains("custom_flag = true"), "app-added key inside a template table survives");
        assert!(out.contains("[model_providers.other]"));
        assert!(out.contains("[projects.\"/Users/x/proj\"]"));
        assert!(out.contains("followUpQueueMode"));
        assert!(out.contains("# Consus Gateway: compliance baseline"));
        assert!(!home.join(".codex/config.toml.consus-bak").exists(), "no snapshot of a key-bearing file");
        assert!(!out.contains("Docs checked"), "the template's own header comment stays in the template");

        let first_table = out.find("\n[").unwrap();
        for key in ["model = ", "model_provider = ", "model_catalog_json", "notify = ["] {
            assert!(out.find(key).unwrap() < first_table, "{key} must sit above the first table");
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn remove_strips_only_the_launchers_keys() {
        let home = temp_home("remove");
        fs::write(config_path(&home), EXISTING).unwrap();
        write_config(&home, &serde_json::from_str(MODELS).unwrap()).unwrap();
        remove_config(&home).unwrap();
        let out = fs::read_to_string(config_path(&home)).unwrap();

        for gone in ["model = ", "model_provider = ", "env_http_headers", "[model_providers.consus]", "[sandbox_workspace_write]", "[otel]"] {
            assert!(!out.contains(gone), "{gone} should be removed");
        }
        for kept in ["[model_providers.other]", "custom_flag = true", "[projects.\"/Users/x/proj\"]", "followUpQueueMode", "notify = [", "model_catalog_json"] {
            assert!(out.contains(kept), "{kept} should survive");
        }
        assert!(!out.contains("computer_use"), "template key inside a shared table is removed");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn fresh_machine_ends_with_no_file() {
        let home = temp_home("fresh");
        write_config(&home, &json!([{ "id": "consus/gpt-5.4:itar" }])).unwrap();
        assert!(config_path(&home).exists());
        remove_config(&home).unwrap();
        assert!(!config_path(&home).exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_non_table_in_the_way_is_an_error_not_a_panic() {
        let home = temp_home("conflict");
        fs::write(config_path(&home), "features = 1\n").unwrap();
        let err = write_config(&home, &json!([{ "id": "consus/gpt-5.4:itar" }])).unwrap_err();
        assert!(err.contains("features"), "{err}");
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
