// The policy document: every value that says what a locked-down tool looks
// like, in one file, policy/launcher-policy.json at the repository root.
// The launcher embeds it here and renders it into each tool's user-space
// settings; the Consus portal renders the same file into the device-
// management lock files, so the two cannot drift. Values only: how a value
// reaches the disk (merging, helper links, model discovery) is the writers'
// business, and anything only they need (library ids, catalog building)
// stays with them.
//
// Parsed once, strictly: a key this code does not know is a build defect,
// the same posture as a template that fails to parse.

// Read by the writers from PR 2 on; until then only the tests use it.
#![allow(dead_code)]

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const SOURCE: &str = include_str!("../../policy/launcher-policy.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: u32,
    pub launcher: String,
    pub about: String,
    pub slots: BTreeMap<String, Slot>,
    pub tools: BTreeMap<String, Tool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub kind: String,
    pub note: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub settings: Map<String, Value>,
    #[serde(default)]
    pub slot_keys: BTreeMap<String, String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub env_slot_keys: BTreeMap<String, String>,
    #[serde(default)]
    pub by_web_access: BTreeMap<String, Map<String, Value>>,
    #[serde(default)]
    pub lock_only: Map<String, Value>,
    pub lock: Option<Value>,
    pub toml: Option<String>,
    #[serde(default)]
    pub toml_slot_keys: BTreeMap<String, String>,
    pub extends: Option<String>,
    pub toml_extra: Option<String>,
    pub readme: Readme,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Readme {
    pub telemetry: String,
    pub web_search: String,
    pub web_fetch: String,
    pub shell: String,
    pub leaves_machine: String,
    pub lock: String,
}

pub fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| serde_json::from_str(SOURCE).expect("policy/launcher-policy.json is valid"))
}

impl Policy {
    pub fn tool(&self, id: &str) -> &Tool {
        self.tools.get(id).unwrap_or_else(|| panic!("policy has no tool {id}"))
    }

    fn web(&self, id: &str, web_access: bool) -> &Map<String, Value> {
        static EMPTY: OnceLock<Map<String, Value>> = OnceLock::new();
        let key = if web_access { "on" } else { "off" };
        self.tool(id).by_web_access.get(key).unwrap_or_else(|| EMPTY.get_or_init(Map::new))
    }

    /// Claude Desktop's static entry keys, as the writer puts them.
    pub fn claude_desktop_static(&self) -> Map<String, Value> {
        self.tool("claude-desktop").settings.clone()
    }

    /// Claude Desktop's keys that follow the org's web-access setting.
    pub fn claude_desktop_variant(&self, web_access: bool) -> Map<String, Value> {
        self.web("claude-desktop", web_access).clone()
    }

    /// Claude Code's static top-level keys (not the slots).
    pub fn claude_code_static(&self) -> Map<String, Value> {
        self.tool("claude-code").settings.clone()
    }

    /// Claude Code's static env values.
    pub fn claude_code_env_static(&self) -> Map<String, Value> {
        self.tool("claude-code").env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect()
    }

    /// Claude Code's web-access variant: extra env, the deny rules, and the
    /// sandbox block (None when the variant has none).
    pub fn claude_code_variant(&self, web_access: bool) -> (Map<String, Value>, Vec<String>, Option<Value>) {
        let v = self.web("claude-code", web_access);
        let env = v.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
        let deny = v
            .get("permissions")
            .and_then(|p| p.get("deny"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        (env, deny, v.get("sandbox").cloned())
    }

    /// The ChatGPT / Codex config.toml, verbatim.
    pub fn chatgpt_toml(&self) -> &str {
        self.tool("chatgpt-desktop").toml.as_deref().expect("chatgpt-desktop carries toml")
    }

    /// The lines only Codex CLI's file carries, verbatim TOML.
    pub fn codex_extra_toml(&self) -> &str {
        self.tool("codex-cli").toml_extra.as_deref().unwrap_or("")
    }

    /// Codex's requirements.toml, the lock, verbatim.
    pub fn codex_requirements_toml(&self) -> &str {
        self.tool("chatgpt-desktop").lock_only.get("requirements_toml").and_then(Value::as_str).unwrap_or("")
    }

    /// Pi's environment, in document order.
    pub fn pi_env(&self) -> Vec<(String, String)> {
        self.tool("pi").env.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Codex's sandbox network value for the org's web-access setting.
    pub fn codex_network_access(&self, web_access: bool) -> bool {
        self.web("chatgpt-desktop", web_access)
            .get("sandbox_workspace_write.network_access")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// Every leaf path of a JSON object, dotted, in sorted order.
pub fn leaf_paths(v: &Value) -> Vec<String> {
    fn walk(v: &Value, prefix: &str, out: &mut Vec<String>) {
        match v.as_object() {
            Some(o) if !o.is_empty() => {
                for (k, child) in o {
                    let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    walk(child, &p, out);
                }
            }
            _ => out.push(prefix.to_string()),
        }
    }
    let mut out = Vec::new();
    walk(v, "", &mut out);
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::DocumentMut;

    #[test]
    fn the_document_parses_and_names_this_launcher() {
        let p = policy();
        assert_eq!(p.schema, 1);
        assert_eq!(p.launcher, env!("CARGO_PKG_VERSION"), "bump policy.launcher with the version");
        let ids: Vec<&str> = p.tools.keys().map(String::as_str).collect();
        assert_eq!(ids, ["chatgpt-desktop", "claude-code", "claude-desktop", "codex-cli", "pi"]);
        for (id, t) in &p.tools {
            assert!(!t.name.is_empty(), "{id} has a name");
            if let Some(base) = &t.extends {
                assert!(p.tools.contains_key(base), "{id} extends {base}");
            }
        }
    }

    #[test]
    fn env_values_are_strings_and_variants_are_on_or_off() {
        let p = policy();
        for (id, t) in &p.tools {
            for (name, v) in &t.by_web_access {
                assert!(name == "on" || name == "off", "{id}: variant {name}");
                if let Some(env) = v.get("env").and_then(Value::as_object) {
                    assert!(env.values().all(Value::is_string), "{id}: variant env values are strings");
                }
            }
        }
        // Claude Code's managed settings reject a non-string env value.
        assert!(p.claude_code_env_static().values().all(Value::is_string));
    }

    #[test]
    fn the_toml_texts_parse_and_keep_top_level_keys_first() {
        let p = policy();
        for (what, text) in [("config.toml", p.chatgpt_toml()), ("requirements.toml", p.codex_requirements_toml())] {
            let doc: DocumentMut = text.parse().unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(!doc.as_table().is_empty(), "{what} has keys");
            // Codex ignores a top-level key written below the first [table].
            let first_table = text.lines().position(|l| l.trim_start().starts_with('['));
            if let Some(ft) = first_table {
                for (i, l) in text.lines().enumerate().skip(ft) {
                    let l = l.trim();
                    if l.is_empty() || l.starts_with('#') || l.starts_with('[') {
                        continue;
                    }
                    let key = l.split('=').next().unwrap().trim().trim_matches('"');
                    assert!(
                        doc.as_table().get(key).map_or(true, |item| !item.is_value()),
                        "{what}: top-level key {key} appears below a table header at line {}",
                        i + 1
                    );
                }
            }
        }
        let extra: DocumentMut = p.codex_extra_toml().parse().unwrap();
        assert_eq!(extra["check_for_update_on_startup"].as_bool(), Some(false));
        assert!(p.codex_requirements_toml().contains("allowed_web_search_modes = []"));
    }

    #[test]
    fn until_the_writers_read_it_the_toml_equals_the_template() {
        // Temporary: removed when chatgpt.rs reads the document (PR 2).
        assert_eq!(policy().chatgpt_toml(), include_str!("../templates/chatgpt-desktop.toml"));
    }

    #[test]
    fn web_access_variants_say_what_the_writers_write_today() {
        let p = policy();
        assert_eq!(p.claude_desktop_variant(false)["coworkEgressAllowedHosts"], serde_json::json!([]));
        assert_eq!(p.claude_desktop_variant(true)["coworkEgressAllowedHosts"], serde_json::json!(["*"]));
        let (env_off, deny_off, sandbox_off) = p.claude_code_variant(false);
        assert_eq!(env_off["CLAUDE_CODE_DISABLE_WEB_FETCH"], "1");
        assert_eq!(deny_off, ["WebSearch", "WebFetch"]);
        assert_eq!(sandbox_off.unwrap()["network"]["strictAllowlist"], true);
        let (env_on, deny_on, sandbox_on) = p.claude_code_variant(true);
        assert!(env_on.is_empty() && sandbox_on.is_none());
        assert_eq!(deny_on, ["WebSearch"]);
        assert!(!p.codex_network_access(false) && p.codex_network_access(true));
        assert_eq!(p.pi_env(), [("PI_SKIP_VERSION_CHECK".to_string(), "1".to_string()), ("PI_TELEMETRY".to_string(), "0".to_string())]);
        assert_eq!(leaf_paths(&serde_json::json!({ "a": { "b": 1, "c": {} }, "d": [] })), ["a.b", "a.c", "d"]);
    }
}
