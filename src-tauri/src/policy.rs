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

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const SOURCE: &str = include_str!("../../policy/launcher-policy.json");

// Fields that describe the document for the portal and the README table
// are read by the tests, not the writers.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct Policy {
    pub schema: u32,
    pub launcher: String,
    pub about: String,
    pub slots: BTreeMap<String, Slot>,
    pub tools: BTreeMap<String, Tool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct Slot {
    pub kind: String,
    pub note: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
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
    pub lock_only: LockOnly,
    pub lock: Option<Lock>,
    pub toml: Option<String>,
    #[serde(default)]
    pub toml_slot_keys: BTreeMap<String, String>,
    pub extends: Option<String>,
    pub toml_extra: Option<String>,
    pub readme: Readme,
}

/// What only a device-management lock may carry.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct LockOnly {
    /// Always in the lock.
    #[serde(default)]
    pub required: Map<String, Value>,
    /// A field on the portal's page, with its default.
    #[serde(default)]
    pub admin: Map<String, Value>,
    /// Keys of `settings` an admin may change in the lock (the lock keeps
    /// the document's value otherwise).
    #[serde(default)]
    pub admin_overrides: Vec<String>,
    /// Codex's requirements.toml, verbatim, with `requirements_slots`
    /// placeholders.
    pub requirements_toml: Option<String>,
    #[serde(default)]
    pub requirements_slots: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct Lock {
    pub kind: String,
    pub domain: Option<String>,
    pub windows_key: Option<String>,
    pub paths: Option<BTreeMap<String, String>>,
    pub note: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
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

    /// The tool whose TOML and variants `id` uses: itself, or the one it
    /// extends.
    pub fn base(&self, id: &str) -> &Tool {
        let t = self.tool(id);
        match &t.extends {
            Some(b) => self.tool(b),
            None => t,
        }
    }

    /// A tool that has web-access variants must have both; one that has
    /// none (Pi) gets an empty map.
    fn web(&self, id: &str, web_access: bool) -> &Map<String, Value> {
        static EMPTY: OnceLock<Map<String, Value>> = OnceLock::new();
        let variants = &self.base(id).by_web_access;
        if variants.is_empty() {
            return EMPTY.get_or_init(Map::new);
        }
        let key = if web_access { "on" } else { "off" };
        variants.get(key).unwrap_or_else(|| panic!("policy: {id} has no {key} web-access variant"))
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
            .map(|a| a.iter().map(|r| r.as_str().expect("permissions.deny holds rule strings").to_string()).collect())
            .unwrap_or_default();
        (env, deny, v.get("sandbox").cloned())
    }

    /// The ChatGPT / Codex config.toml, verbatim.
    pub fn chatgpt_toml(&self) -> &str {
        self.base("codex-cli").toml.as_deref().expect("chatgpt-desktop carries toml")
    }

    /// The lines only Codex CLI's file carries, verbatim TOML.
    pub fn codex_extra_toml(&self) -> &str {
        self.tool("codex-cli").toml_extra.as_deref().unwrap_or("")
    }

    /// Codex's requirements.toml, the lock, verbatim, with its slot
    /// placeholders ({{endpoint_host}}) still in place. The portal's, not
    /// the launcher's, to render; the tests check it parses.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn codex_requirements_toml(&self) -> &str {
        self.base("codex-cli").lock_only.requirements_toml.as_deref().unwrap_or("")
    }

    /// Pi's environment, sorted by name (the order has no meaning).
    pub fn pi_env(&self) -> Vec<(String, String)> {
        self.tool("pi").env.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Codex's sandbox network value for the org's web-access setting.
    pub fn codex_network_access(&self, web_access: bool) -> bool {
        self.web("codex-cli", web_access)
            .get("sandbox_workspace_write")
            .and_then(|s| s.get("network_access"))
            .and_then(Value::as_bool)
            .expect("the Codex variant sets sandbox_workspace_write.network_access")
    }
}

/// Every leaf path of a JSON object, dotted, in sorted order.
#[cfg_attr(not(test), allow(dead_code))]
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
        // Env values travel into a shell line (Pi) and a JSON file: plain text.
        let plain = |v: &str| v.chars().all(|c| c.is_ascii_alphanumeric() || "._-/:".contains(c));
        for (id, t) in &p.tools {
            for (k, v) in &t.env {
                assert!(plain(v), "{id}: {k}={v}");
            }
            for (name, variant) in &t.by_web_access {
                for (k, v) in variant.get("env").and_then(Value::as_object).into_iter().flatten() {
                    assert!(v.as_str().is_some_and(plain), "{id} {name}: {k}={v}");
                }
            }
        }
        for on in [false, true] {
            assert!(!p.claude_desktop_variant(on).is_empty() && !p.claude_code_variant(on).1.is_empty());
        }
    }

    /// Codex's top-level keys. One of these written below a [table] header
    /// is silently read as a key of that table, so the text must keep them
    /// above the first header; toml_edit alone cannot tell.
    const CODEX_TOP_LEVEL: [&str; 10] = [
        "model",
        "model_provider",
        "model_reasoning_effort",
        "model_reasoning_summary",
        "web_search",
        "approval_policy",
        "approvals_reviewer",
        "sandbox_mode",
        "check_for_update_on_startup",
        "model_catalog_json",
    ];

    fn keys_below_first_header(text: &str) -> Vec<String> {
        let mut below = false;
        let mut out = Vec::new();
        for l in text.lines() {
            let l = l.trim();
            if l.starts_with('[') {
                below = true;
                continue;
            }
            if below && !l.is_empty() && !l.starts_with('#') {
                out.push(l.split('=').next().unwrap().trim().trim_matches('"').to_string());
            }
        }
        out
    }

    #[test]
    fn the_toml_texts_parse_and_keep_top_level_keys_first() {
        let p = policy();
        let req = p.codex_requirements_toml().replace("{{endpoint_host}}", "api.consus.io");
        for (what, text) in [("config.toml", p.chatgpt_toml().to_string()), ("requirements.toml", req)] {
            let doc: DocumentMut = text.parse().unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(!doc.as_table().is_empty(), "{what} has keys");
        }
        for key in keys_below_first_header(p.chatgpt_toml()) {
            assert!(!CODEX_TOP_LEVEL.contains(&key.as_str()), "config.toml: top-level key {key} sits below a table header");
        }
        // The check itself works: a misplaced key is caught.
        let bad = "model = \"x\"\n[sandbox_workspace_write]\nweb_search = \"disabled\"\n";
        assert!(keys_below_first_header(bad).iter().any(|k| CODEX_TOP_LEVEL.contains(&k.as_str())));
        // Every top-level key the file sets is one Codex knows at top level.
        let doc: DocumentMut = p.chatgpt_toml().parse().unwrap();
        for (k, item) in doc.as_table().iter() {
            if item.is_value() {
                assert!(CODEX_TOP_LEVEL.contains(&k), "config.toml: unknown top-level key {k}; add it to CODEX_TOP_LEVEL if Codex reads it there");
            }
        }
        let extra: DocumentMut = p.codex_extra_toml().parse().unwrap();
        assert_eq!(extra["check_for_update_on_startup"].as_bool(), Some(false));
        // codex.rs copies these as plain values; a table or dotted key here
        // would panic at launch.
        assert!(extra.as_table().iter().all(|(_, item)| item.is_value()), "toml_extra holds plain values only");
        let lock = p.base("codex-cli").lock_only.requirements_toml.as_deref().unwrap();
        assert!(lock.contains("allowed_web_search_modes = []"));
        assert!(lock.contains("\"{{endpoint_host}}\" = \"allow\""), "the gateway host is a slot, not api.consus.io");
        assert_eq!(p.base("codex-cli").lock_only.requirements_slots.get("{{endpoint_host}}").map(String::as_str), Some("endpoint_host"));
        assert!(p.slots.contains_key("endpoint_host"));
    }

    /// The README's per-tool table, rendered from the document. Rows in
    /// the order people read them, not the map's.
    fn readme_table() -> String {
        let p = policy();
        let mut out = String::from(
            "| | Inference | Vendor telemetry and updates | Web search | Web fetch, off / on | Agent shell network, off / on | What still leaves the machine | Lock it for the user |\n|---|---|---|---|---|---|---|---|\n",
        );
        for id in ["claude-desktop", "claude-code", "chatgpt-desktop", "codex-cli", "pi"] {
            let t = p.tool(id);
            let r = &t.readme;
            out.push_str(&format!(
                "| {} | Gateway only | {} | {} | {} | {} | {} | {} |\n",
                t.name, r.telemetry, r.web_search, r.web_fetch, r.shell, r.leaves_machine, r.lock
            ));
        }
        out
    }

    #[test]
    fn the_readme_table_is_the_document() {
        // A Windows checkout has CRLF line endings; the table is compared as LF.
        let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md")).unwrap().replace("\r\n", "\n");
        let (start_tag, end_tag) = ("<!-- lockdown-table:start", "<!-- lockdown-table:end -->");
        let start = readme.find(start_tag).expect("README has the table's start marker");
        let body_start = start + readme[start..].find("-->\n").expect("start marker closes") + 4;
        let end = readme.find(end_tag).expect("README has the table's end marker");
        let current = &readme[body_start..end];
        let rendered = readme_table();
        assert!(
            current == rendered,
            "README's per-tool table differs from policy/launcher-policy.json. Paste this between the markers:\n\n{rendered}"
        );
    }

    #[test]
    fn lock_only_is_typed_and_does_not_restate_settings() {
        let p = policy();
        let cd = p.tool("claude-desktop");
        for k in cd.lock_only.admin.keys() {
            assert!(!cd.settings.contains_key(k), "claude-desktop: {k} is in settings; list it under admin_overrides instead");
        }
        for k in &cd.lock_only.admin_overrides {
            assert!(cd.settings.contains_key(k), "claude-desktop: admin_overrides names {k}, which settings lacks");
        }
        let cc = p.tool("claude-code");
        assert_eq!(cc.lock_only.required["allowedProviders"], serde_json::json!(["customEndpoint"]));
        assert_eq!(leaf_paths(&Value::Object(cc.lock_only.admin.clone())), ["allowManagedMcpServersOnly", "allowedMcpServers", "sandbox.network.allowManagedDomainsOnly"]);
        assert_eq!(p.tool("codex-cli").extends.as_deref(), Some("chatgpt-desktop"));
        assert!(p.tool("codex-cli").toml.is_none(), "codex-cli takes its TOML from chatgpt-desktop");
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
        assert_eq!(p.pi_env(), [("PI_SKIP_VERSION_CHECK".to_string(), "1".to_string()), ("PI_TELEMETRY".to_string(), "0".to_string())], "sorted by name");
        assert_eq!(leaf_paths(&serde_json::json!({ "a": { "b": 1, "c": {} }, "d": [] })), ["a.b", "a.c", "d"]);
    }
}
