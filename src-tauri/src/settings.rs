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

/// A string setting. Absent is None. Blank is None only where that cannot
/// silently change where requests go; for the others it is an error.
fn string(v: Option<Value>, key: &str, blank_ok: bool) -> Result<Option<String>, String> {
    match v {
        None => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => {
            if blank_ok {
                Ok(None)
            } else {
                Err(format!("{key} is set but empty."))
            }
        }
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("{key} must be a string.")),
    }
}

/// "https://proxy.example.com/" or ".../v1/" -> "https://proxy.example.com".
/// https only, except plain http to this machine for a local proxy. No
/// query, fragment, credentials, spaces, or quotes: the value is written
/// into several tools' config files.
pub fn normalize_endpoint(raw: &str) -> Result<String, String> {
    let bad = || format!("EndpointURL {raw:?} is not a valid https URL.");
    let mut s = raw.trim().to_string();
    loop {
        let before = s.len();
        s.truncate(s.trim_end_matches('/').len());
        if let Some(i) = s.len().checked_sub(3).filter(|&i| s.is_char_boundary(i)) {
            if s[i..].eq_ignore_ascii_case("/v1") {
                s.truncate(i);
            }
        }
        if s.len() == before {
            break;
        }
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control() || "\"'\\`<>?#@{}|^".contains(c)) {
        return Err(bad());
    }
    let rest = if let Some(r) = s.strip_prefix("https://") {
        r
    } else if let Some(r) = s.strip_prefix("http://") {
        let host = r.split(['/', ':']).next().unwrap_or("");
        if host != "localhost" && host != "127.0.0.1" {
            return Err(format!(
                "EndpointURL {raw:?} must use https (plain http is allowed only to localhost or 127.0.0.1)."
            ));
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
    let endpoint = match string(raw.endpoint, "EndpointURL", false)? {
        Some(e) => normalize_endpoint(&e)?,
        None => DEFAULT_ENDPOINT.to_string(),
    };
    let level = match string(raw.level, "ComplianceLevel", false)? {
        Some(l) if valid_level(&l) => l,
        Some(l) => return Err(format!("ComplianceLevel {l:?} is not a compliance level the gateway knows.")),
        None => DEFAULT_LEVEL.to_string(),
    };
    let listed: Option<Vec<String>> = match raw.tools {
        None => None,
        Some(Value::String(s)) => Some(s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()),
        Some(Value::Array(a)) => Some(
            a.iter()
                .map(|x| x.as_str().map(|s| s.trim().to_string()).ok_or("Tools must list tool ids as strings."))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err("Tools must be a list of tool ids.".into()),
    };
    let tools = match listed {
        None => None,
        Some(ids) => {
            // Unknown ids are skipped, so a newer tool id cannot break an
            // older launcher; but a list with no id this launcher knows is
            // a typo, not "no tools".
            let keys: Vec<String> = ids
                .iter()
                .filter_map(|id| TOOL_IDS.iter().find(|(pid, _)| pid == id).map(|(_, key)| key.to_string()))
                .collect();
            if keys.is_empty() && !ids.is_empty() {
                let known: Vec<&str> = TOOL_IDS.iter().map(|(pid, _)| *pid).collect();
                return Err(format!(
                    "Tools lists no tool this launcher knows ({}). Use: {}.",
                    ids.join(", "),
                    known.join(", ")
                ));
            }
            Some(keys)
        }
    };
    Ok(Settings {
        target: Target { endpoint, level },
        tools,
        org_name: string(raw.org_name, "OrgName", true)?,
        managed: raw.managed,
    })
}

/// One key from a plist, via the system's plutil. Arrays come out as JSON;
/// plutil will not write a lone string as JSON, so plain values come out
/// raw and are read as strings. The file was already checked to be readable
/// (see `read`), so a failure here means the key is not set.
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

/// Where the values come from, first match wins, as macOS itself orders
/// them: the user's managed preferences, the machine's, then the user's own
/// preferences, then the machine's (`sudo defaults write /Library/Preferences/...`).
fn sources(home: &Path) -> Vec<(PathBuf, bool)> {
    let file = format!("{DOMAIN}.plist");
    let managed = Path::new("/Library/Managed Preferences");
    let user = std::env::var("USER").ok().or_else(|| home.file_name().map(|n| n.to_string_lossy().into_owned()));
    let mut out = Vec::new();
    if let Some(user) = user {
        out.push((managed.join(user).join(&file), true));
    }
    out.push((managed.join(&file), true));
    out.push((home.join("Library/Preferences").join(&file), false));
    out.push((Path::new("/Library/Preferences").join(&file), false));
    out
}

/// A settings file that exists but cannot be read is an error, not an
/// empty file: treating it as empty would send every tool to the defaults.
fn read(home: &Path) -> Result<Raw, (String, bool)> {
    read_from(sources(home))
}

fn read_from(sources: Vec<(PathBuf, bool)>) -> Result<Raw, (String, bool)> {
    let mut raw = Raw::default();
    for (path, is_managed) in sources {
        if !path.exists() {
            continue;
        }
        let lint = Command::new("plutil").arg("-lint").arg(&path).output();
        match lint {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err((format!("{} could not be read ({why}).", path.display()), is_managed));
            }
            Err(e) => return Err((format!("{} could not be checked ({e}).", path.display()), is_managed)),
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
    Ok(raw)
}

/// Windows keeps the same keys as registry values, first match wins: the
/// machine's policy (what Intune and Group Policy write), the user's policy,
/// then the user's own. Tools may be REG_MULTI_SZ or a comma-separated REG_SZ.
#[cfg_attr(not(windows), allow(dead_code))]
const REG_SOURCES: [(&str, bool); 3] = [
    ("HKLM\\SOFTWARE\\Policies\\Consus\\Launcher", true),
    ("HKCU\\SOFTWARE\\Policies\\Consus\\Launcher", true),
    ("HKCU\\Software\\Consus\\Launcher", false),
];

/// One value out of `reg query <key> /v <name>` output, whose value lines
/// read "    Name    TYPE    data". A REG_MULTI_SZ prints its parts
/// joined by a literal \0. Any other type is kept as text, so a value of
/// the wrong type is reported as invalid rather than skipped.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_reg(text: &str, name: &str) -> Option<Value> {
    text.lines().find_map(|line| {
        let mut parts = line.trim_start().splitn(3, "    ");
        let (n, kind, data) = (parts.next()?, parts.next()?, parts.next().unwrap_or("").trim_end());
        if !n.eq_ignore_ascii_case(name) || !kind.starts_with("REG_") {
            return None;
        }
        Some(if kind == "REG_MULTI_SZ" {
            Value::Array(data.split("\\0").filter(|p| !p.is_empty()).map(|p| Value::String(p.into())).collect())
        } else {
            Value::String(data.into())
        })
    })
}

#[cfg(windows)]
fn read_registry() -> Raw {
    use std::os::windows::process::CommandExt;
    let mut raw = Raw::default();
    for (key, is_managed) in REG_SOURCES {
        let slots: [(&str, &mut Option<Value>); 4] = [
            ("EndpointURL", &mut raw.endpoint),
            ("ComplianceLevel", &mut raw.level),
            ("Tools", &mut raw.tools),
            ("OrgName", &mut raw.org_name),
        ];
        for (name, slot) in slots {
            if slot.is_some() {
                continue;
            }
            let out = Command::new("reg").args(["query", key, "/v", name]).creation_flags(0x0800_0000).output();
            let found = out
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| parse_reg(&String::from_utf8_lossy(&o.stdout), name));
            if let Some(v) = found {
                *slot = Some(v);
                raw.managed |= is_managed;
            }
        }
    }
    raw
}

#[cfg(not(windows))]
fn read_registry() -> Raw {
    Raw::default()
}

/// The settings for this run, and whether device management supplied any
/// of them (so an error can say who can fix it).
static SETTINGS: OnceLock<(Result<Settings, String>, bool)> = OnceLock::new();

fn load() -> &'static (Result<Settings, String>, bool) {
    SETTINGS.get_or_init(|| {
        let read = if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|h| read(Path::new(&h))).unwrap_or(Ok(Raw::default()))
        } else {
            Ok(read_registry())
        };
        let (result, managed) = match read {
            Err((e, managed)) => (Err(e), managed),
            Ok(raw) => {
                let managed = raw.managed;
                (parse(raw), managed)
            }
        };
        let who = if managed {
            "Your organization's launcher settings"
        } else {
            "The launcher settings in your own preferences"
        };
        (result.map_err(|e| format!("{who} are invalid: {e}")), managed)
    })
}

/// The settings for this run of the launcher, read on first use; restart the
/// launcher to pick up a change. Other platforms get the defaults until they
/// have a settings source.
pub fn current() -> Result<Settings, String> {
    load().0.clone()
}

#[derive(Serialize)]
pub struct SettingsView {
    pub org_name: Option<String>,
    pub managed: bool,
    pub error: Option<String>,
}

/// Async so the first read (a few plutil runs) happens off the UI thread.
#[tauri::command]
pub async fn get_settings() -> SettingsView {
    let (result, managed) = load();
    match result {
        Ok(s) => SettingsView { org_name: s.org_name.clone(), managed: *managed, error: None },
        Err(e) => SettingsView { org_name: None, managed: *managed, error: Some(e.clone()) },
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
        // Blank would silently mean api.consus.io or ITAR.
        assert!(parse(raw(Some(json!("  ")), None, None)).is_err());
        assert!(parse(raw(None, Some(json!("")), None)).is_err());
        // A list with no known tool is a typo, not "no tools".
        let err = parse(raw(None, None, Some(json!("claude_code, codex_cli")))).unwrap_err();
        assert!(err.contains("claude_code"), "{err}");
        // An empty list does mean no tools.
        assert_eq!(parse(raw(None, None, Some(json!([])))).unwrap().tools, Some(vec![]));
    }

    #[test]
    fn v1_is_stripped_however_it_is_written() {
        for (input, out) in [
            ("https://x.example//v1", "https://x.example"),
            ("https://x.example/V1/", "https://x.example"),
            ("https://x.example/v1/v1", "https://x.example"),
            ("https://x.example/consus/v1", "https://x.example/consus"),
        ] {
            assert_eq!(normalize_endpoint(input).unwrap(), out, "{input}");
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn an_unreadable_settings_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("consus-launcher-badplist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("io.consus.launcher.plist");
        std::fs::write(&path, "<plist><dict><key>EndpointURL</key><string>https://proxy").unwrap();
        let err = read_from(vec![(path.clone(), true)]).err().expect("a truncated file must not read as empty");
        assert!(err.1, "the error remembers the file was managed");
        assert!(read_from(vec![(dir.join("absent.plist"), true)]).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn reads_registry_values() {
        let out = "\r\nHKEY_CURRENT_USER\\Software\\Consus\\Launcher\r\n    OrgName    REG_SZ    Acme Test\r\n\r\n";
        assert_eq!(parse_reg(out, "OrgName"), Some(json!("Acme Test")));
        assert_eq!(parse_reg(out, "orgname"), Some(json!("Acme Test")));
        assert_eq!(parse_reg(out, "Tools"), None);
        let multi = "HKEY_LOCAL_MACHINE\\SOFTWARE\\Policies\\Consus\\Launcher\r\n    Tools    REG_MULTI_SZ    claude-code\\0codex-cli\r\n";
        assert_eq!(parse_reg(multi, "Tools"), Some(json!(["claude-code", "codex-cli"])));
        let dword = "    ComplianceLevel    REG_DWORD    0x1\r\n";
        assert_eq!(parse_reg(dword, "ComplianceLevel"), Some(json!("0x1")), "a wrong type is kept, so it reads as invalid");
        let url = "    EndpointURL    REG_SZ    https://proxy.acme.example/v1\r\n";
        assert_eq!(parse_reg(url, "EndpointURL"), Some(json!("https://proxy.acme.example/v1")));
    }
}
