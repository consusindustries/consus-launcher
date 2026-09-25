// Org settings: what an admin decided for this machine. Read once at
// startup from macOS managed preferences for io.consus.launcher (the
// configuration profile device management pushes), then the user's own
// preferences for that domain, then built-in defaults. Keys, all optional:
//
//   EndpointURL      where every tool points, e.g. https://api.consus.io or
//                    the org's logging proxy
//   ComplianceLevel  the model id suffix: itar, fedramp-high, fedramp-high+itar
//   Tools            the tools shown, as an array or a comma-separated string
//                    of: claude-desktop, chatgpt-desktop, claude-code,
//                    codex-cli, pi
//   OrgName          shown in the launcher's header
//
// A value that is present but invalid is an error, never a silent default:
// falling back to api.consus.io would quietly bypass an org's proxy.
// Settings shape what the launcher shows; the gateway is what enforces.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::models::{valid_level, Target, DEFAULT_ENDPOINT, DEFAULT_LEVEL};

pub const DOMAIN: &str = "io.consus.launcher";

/// The portal's tool ids, and the launcher's key for each.
const TOOL_IDS: [(&str, &str); 5] = [
    ("claude-desktop", "desktop"),
    ("chatgpt-desktop", "chatgpt"),
    ("claude-code", "code"),
    ("codex-cli", "codex"),
    ("pi", "pi"),
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Settings {
    #[serde(skip)]
    pub target: Target,
    /// Launcher keys of the tools the org allows; None allows every tool.
    pub tools: Option<Vec<String>>,
    pub org_name: Option<String>,
    /// Whether any value came from device management.
    pub managed: bool,
}

impl Settings {
    pub fn allows(&self, tool_key: &str) -> bool {
        self.tools.as_ref().is_none_or(|t| t.iter().any(|k| k == tool_key))
    }
}

/// Values as found, before validation.
#[derive(Default)]
struct Raw {
    endpoint: Option<Value>,
    level: Option<Value>,
    tools: Option<Value>,
    org_name: Option<Value>,
    managed: bool,
}

fn string(v: Option<Value>, key: &str) -> Result<Option<String>, String> {
    match v {
        None => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("{key} must be a string.")),
    }
}

/// "https://proxy.example.com/" or ".../v1" -> "https://proxy.example.com".
/// https only, except plain http to this machine for a local proxy. No
/// query, fragment, credentials, spaces, or quotes: the value is written
/// into several tools' config files.
pub fn normalize_endpoint(raw: &str) -> Result<String, String> {
    let bad = || format!("EndpointURL {raw:?} is not a valid https URL.");
    let mut s = raw.trim().trim_end_matches('/').to_string();
    if let Some(stripped) = s.strip_suffix("/v1") {
        s = stripped.to_string();
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control() || "\"'\\`<>?#@{}|^".contains(c)) {
        return Err(bad());
    }
    let rest = if let Some(r) = s.strip_prefix("https://") {
        r
    } else if let Some(r) = s.strip_prefix("http://") {
        let host = r.split(['/', ':']).next().unwrap_or("");
        if host != "localhost" && host != "127.0.0.1" {
            return Err(format!("EndpointURL {raw:?} must use https (plain http is allowed only to localhost)."));
        }
        r
    } else {
        return Err(bad());
    };
    let host = rest.split(['/', ':']).next().unwrap_or("");
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err(bad());
    }
    Ok(s)
}

fn parse(raw: Raw) -> Result<Settings, String> {
    let endpoint = match string(raw.endpoint, "EndpointURL")? {
        Some(e) => normalize_endpoint(&e)?,
        None => DEFAULT_ENDPOINT.to_string(),
    };
    let level = match string(raw.level, "ComplianceLevel")? {
        Some(l) if valid_level(&l) => l,
        Some(l) => return Err(format!("ComplianceLevel {l:?} is not a compliance level the gateway knows.")),
        None => DEFAULT_LEVEL.to_string(),
    };
    let tools = match raw.tools {
        None => None,
        Some(Value::String(s)) => Some(s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()),
        Some(Value::Array(a)) => Some(
            a.iter()
                .map(|x| x.as_str().map(|s| s.trim().to_string()).ok_or("Tools must list tool ids as strings."))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err("Tools must be a list of tool ids.".into()),
    }
    .map(|ids: Vec<String>| {
        // Unknown ids are skipped, so a newer tool id cannot break an older launcher.
        ids.iter()
            .filter_map(|id| TOOL_IDS.iter().find(|(pid, _)| pid == id).map(|(_, key)| key.to_string()))
            .collect()
    });
    Ok(Settings {
        target: Target { endpoint, level },
        tools,
        org_name: string(raw.org_name, "OrgName")?,
        managed: raw.managed,
    })
}

/// One key from a plist, via the system's plutil. Arrays come out as JSON;
/// plutil will not write a lone string as JSON, so plain values come out
/// raw and are read as strings.
fn plist_value(path: &Path, key: &str) -> Option<Value> {
    let extract = |format: &str| {
        let out = Command::new("plutil").args(["-extract", key, format, "-o", "-"]).arg(path).output().ok()?;
        out.status.success().then_some(out.stdout)
    };
    if let Some(json) = extract("json") {
        return serde_json::from_slice(&json).ok();
    }
    let raw = extract("raw")?;
    Some(Value::String(String::from_utf8_lossy(&raw).trim_end_matches('\n').to_string()))
}

/// Where the values come from, first match wins: the user's managed
/// preferences, the machine's, then the user's own preferences.
fn sources(home: &Path) -> Vec<(PathBuf, bool)> {
    let file = format!("{DOMAIN}.plist");
    let managed = Path::new("/Library/Managed Preferences");
    let mut out = Vec::new();
    if let Ok(user) = std::env::var("USER") {
        out.push((managed.join(user).join(&file), true));
    }
    out.push((managed.join(&file), true));
    out.push((home.join("Library/Preferences").join(&file), false));
    out
}

fn read(home: &Path) -> Raw {
    let mut raw = Raw::default();
    for (path, is_managed) in sources(home) {
        if !path.exists() {
            continue;
        }
        let slots: [(&str, &mut Option<Value>); 4] = [
            ("EndpointURL", &mut raw.endpoint),
            ("ComplianceLevel", &mut raw.level),
            ("Tools", &mut raw.tools),
            ("OrgName", &mut raw.org_name),
        ];
        for (key, slot) in slots {
            if slot.is_none() {
                if let Some(v) = plist_value(&path, key) {
                    *slot = Some(v);
                    raw.managed |= is_managed;
                }
            }
        }
    }
    raw
}

static SETTINGS: OnceLock<Result<Settings, String>> = OnceLock::new();

/// The settings for this run of the launcher, read on first use. Other
/// platforms get the defaults until they have a settings source.
pub fn current() -> Result<Settings, String> {
    SETTINGS
        .get_or_init(|| {
            let raw = if cfg!(target_os = "macos") {
                std::env::var_os("HOME").map(|h| read(Path::new(&h))).unwrap_or_default()
            } else {
                Raw::default()
            };
            parse(raw).map_err(|e| format!("Your organization's launcher settings are invalid: {e}"))
        })
        .clone()
}

#[derive(Serialize)]
pub struct SettingsView {
    pub org_name: Option<String>,
    pub managed: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub fn get_settings() -> SettingsView {
    match current() {
        Ok(s) => SettingsView { org_name: s.org_name, managed: s.managed, error: None },
        Err(e) => SettingsView { org_name: None, managed: true, error: Some(e) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(endpoint: Option<Value>, level: Option<Value>, tools: Option<Value>) -> Raw {
        Raw { endpoint, level, tools, org_name: None, managed: true }
    }

    #[test]
    fn nothing_set_means_defaults() {
        let s = parse(Raw::default()).unwrap();
        assert_eq!(s.target, Target::default());
        assert_eq!(s.tools, None);
        assert!(s.allows("desktop") && s.allows("pi"));
        assert!(!s.managed);
    }

    #[test]
    fn values_are_applied() {
        let s = parse(raw(
            Some(json!("https://ai-proxy.acme.example/v1/")),
            Some(json!("fedramp-high+itar")),
            Some(json!(["claude-code", "codex-cli", "some-future-tool"])),
        ))
        .unwrap();
        assert_eq!(s.target.endpoint, "https://ai-proxy.acme.example");
        assert_eq!(s.target.v1(), "https://ai-proxy.acme.example/v1");
        assert_eq!(s.target.level, "fedramp-high+itar");
        assert_eq!(s.tools.as_deref(), Some(&["code".to_string(), "codex".to_string()][..]));
        assert!(s.allows("code") && !s.allows("desktop"));
    }

    #[test]
    fn tools_as_a_comma_separated_string() {
        let s = parse(raw(None, None, Some(json!(" pi , claude-desktop ")))).unwrap();
        assert_eq!(s.tools.as_deref(), Some(&["pi".to_string(), "desktop".to_string()][..]));
    }

    #[test]
    fn endpoints() {
        for ok in [
            "https://api.consus.io",
            "https://proxy.acme.example:8443/consus",
            "http://localhost:4000",
            "http://127.0.0.1:4000/",
        ] {
            assert!(normalize_endpoint(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://proxy.acme.example",
            "ftp://x",
            "api.consus.io",
            "https://",
            "https://a b.example",
            "https://x.example/?q=1",
            "https://user:pass@x.example",
            "https://x.example/\"",
            "http://localhost.evil.example",
        ] {
            assert!(normalize_endpoint(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn invalid_values_are_errors_not_defaults() {
        assert!(parse(raw(Some(json!("http://proxy.example")), None, None)).is_err());
        assert!(parse(raw(Some(json!(42)), None, None)).is_err());
        assert!(parse(raw(None, Some(json!("cui")), None)).is_err());
        assert!(parse(raw(None, None, Some(json!({"a": 1})))).is_err());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn reads_strings_and_arrays_from_a_real_plist() {
        let dir = std::env::temp_dir().join(format!("consus-launcher-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("io.consus.launcher.plist");
        std::fs::write(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>OrgName</key><string>Acme Test</string>
<key>EndpointURL</key><string>https://ai-proxy.acme.example</string>
<key>Tools</key><array><string>claude-code</string><string>codex-cli</string></array>
<key>SomeDate</key><date>2026-09-24T10:00:00Z</date>
</dict></plist>"#,
        )
        .unwrap();
        assert_eq!(plist_value(&path, "OrgName"), Some(json!("Acme Test")));
        assert_eq!(plist_value(&path, "EndpointURL"), Some(json!("https://ai-proxy.acme.example")));
        assert_eq!(plist_value(&path, "Tools"), Some(json!(["claude-code", "codex-cli"])));
        assert_eq!(plist_value(&path, "Missing"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
