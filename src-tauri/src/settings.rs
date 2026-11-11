// Org settings: what an admin decided for this machine. Each value is
// decided on its own, from the first of these that sets it:
//
//   1. device management: macOS managed preferences for io.consus.launcher
//      (the configuration profile IT pushes), or the Windows policy keys
//   2. the org's document from the Consus portal, read with the person's key
//      (portal::sync_device_policy), with a last good copy on disk
//   3. this computer's own preferences for that domain
//   4. built-in defaults
//
// Device management and this computer's preferences are read once at
// startup; the document is replaced whenever a read brings a new one. Keys,
// all optional (the document's names are in brackets):
//
//   EndpointURL      [endpoint] where every tool points, e.g.
//                    https://api.consus.io or the org's logging proxy
//   ComplianceLevel  [level] the model id suffix: itar, fedramp-high,
//                    fedramp-high+itar
//   Tools            [tools] the tools shown, as an array or a
//                    comma-separated string of: claude-desktop,
//                    chatgpt-desktop, claude-code, codex-cli, pi
//   OrgName          [org_name] shown in the launcher's header
//   WebAccess        [web_access] whether a tool's agent may fetch web pages
//                    from the machine: off (default) or on
//   UpdateNotice     (device management and this computer only)
//
// A value that is present but invalid is an error, never a silent default:
// falling back to api.consus.io would quietly bypass an org's proxy. The
// document follows the same rule wherever a fallback could loosen what the
// admin chose (see check_document). Settings shape what the launcher shows;
// the gateway is what enforces.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{OnceLock, PoisonError, RwLock};

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

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub target: Target,
    /// Launcher keys of the tools the org allows; None allows every tool.
    pub tools: Option<Vec<String>>,
    pub org_name: Option<String>,
    /// Whether to tell the user a newer launcher exists. Off by default on a
    /// managed machine, where IT pushes updates.
    pub update_notice: bool,
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
    update_notice: Option<Value>,
    web_access: Option<Value>,
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

/// A yes/no setting: a plist boolean, or as text (a registry REG_DWORD reads
/// "0x1"; "on" and "off" read as people write them). Anything else is an
/// error, like every other invalid value.
fn flag(v: Option<Value>, key: &str) -> Result<Option<bool>, String> {
    let bad = || Err(format!("{key} must be on or off (true or false)."));
    match v {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(b)),
        Some(Value::Number(n)) => match n.as_u64() {
            Some(0) => Ok(Some(false)),
            Some(1) => Ok(Some(true)),
            _ => bad(),
        },
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "0x1" | "yes" | "on" => Ok(Some(true)),
            "false" | "0" | "0x0" | "no" | "off" => Ok(Some(false)),
            _ => bad(),
        },
        Some(_) => bad(),
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

/// EndpointURL as the tools get it: normalized, or the default when unset.
fn endpoint_of(v: Option<Value>) -> Result<String, String> {
    match string(v, "EndpointURL", false)? {
        Some(e) => normalize_endpoint(&e),
        None => Ok(DEFAULT_ENDPOINT.to_string()),
    }
}

/// The ids in a Tools value: an array of strings, or one comma-separated
/// string.
fn tool_ids(v: Value) -> Result<Vec<String>, String> {
    match v {
        Value::String(s) => Ok(s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()),
        Value::Array(a) => a
            .iter()
            .map(|x| x.as_str().map(|s| s.trim().to_string()).ok_or_else(|| "Tools must list tool ids as strings.".to_string()))
            .collect(),
        _ => Err("Tools must be a list of tool ids.".into()),
    }
}

/// The launcher keys for the ids this launcher knows. Unknown ids are
/// skipped, so a newer tool id cannot break an older launcher.
fn tool_keys(ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter_map(|id| TOOL_IDS.iter().find(|(pid, _)| pid == id).map(|(_, key)| key.to_string()))
        .collect()
}

fn parse(raw: Raw) -> Result<Settings, String> {
    let endpoint = endpoint_of(raw.endpoint)?;
    let level = match string(raw.level, "ComplianceLevel", false)? {
        Some(l) if valid_level(&l) => l,
        Some(l) => return Err(format!("ComplianceLevel {l:?} is not a compliance level the gateway knows.")),
        None => DEFAULT_LEVEL.to_string(),
    };
    let tools = match raw.tools {
        None => None,
        Some(v) => {
            let ids = tool_ids(v)?;
            let keys = tool_keys(&ids);
            // A list with no id this launcher knows is a typo, not "no tools".
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
        target: Target { endpoint, level, web_access: flag(raw.web_access, "WebAccess")?.unwrap_or(false) },
        tools,
        org_name: string(raw.org_name, "OrgName", true)?,
        update_notice: flag(raw.update_notice, "UpdateNotice")?.unwrap_or(!raw.managed),
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

/// Device management's values and this computer's, apart, so the portal's
/// document can sit between them. A settings file that exists but cannot be
/// read is an error, not an empty file: treating it as empty would send
/// every tool to the defaults.
fn read(home: &Path) -> Result<(Raw, Raw), (String, bool)> {
    let (managed, local): (Vec<_>, Vec<_>) = sources(home).into_iter().partition(|(_, is_managed)| *is_managed);
    Ok((read_from(managed)?, read_from(local)?))
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
        let slots: [(&str, &mut Option<Value>); 6] = [
            ("EndpointURL", &mut raw.endpoint),
            ("ComplianceLevel", &mut raw.level),
            ("Tools", &mut raw.tools),
            ("OrgName", &mut raw.org_name),
            ("UpdateNotice", &mut raw.update_notice),
            ("WebAccess", &mut raw.web_access),
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

/// Device management's values and this computer's, apart, as `read` does.
fn read_registry() -> (Raw, Raw) {
    (read_registry_from(true), read_registry_from(false))
}

#[cfg(windows)]
fn read_registry_from(managed: bool) -> Raw {
    use std::os::windows::process::CommandExt;
    let mut raw = Raw::default();
    for (key, is_managed) in REG_SOURCES.into_iter().filter(|(_, m)| *m == managed) {
        let slots: [(&str, &mut Option<Value>); 6] = [
            ("EndpointURL", &mut raw.endpoint),
            ("ComplianceLevel", &mut raw.level),
            ("Tools", &mut raw.tools),
            ("OrgName", &mut raw.org_name),
            ("UpdateNotice", &mut raw.update_notice),
            ("WebAccess", &mut raw.web_access),
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
fn read_registry_from(_managed: bool) -> Raw {
    Raw::default()
}

/// Where one value came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Mdm,
    Portal,
    Local,
    #[default]
    Default,
}

/// Where each of the five values came from.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Sources {
    pub endpoint: Source,
    pub level: Source,
    pub tools: Source,
    pub org_name: Source,
    pub web_access: Source,
}

impl Sources {
    /// Whether any of the five came from `s`.
    fn any(&self, s: Source) -> bool {
        [self.endpoint, self.level, self.tools, self.org_name, self.web_access].contains(&s)
    }
}

/// Each value from the first of device management, the document, and this
/// computer that sets it. UpdateNotice is not in the document, and whether
/// the machine is managed is device management's alone.
fn merge(mdm: &Raw, doc: &Raw, local: &Raw) -> (Raw, Sources) {
    let pick = |f: fn(&Raw) -> &Option<Value>| {
        [(mdm, Source::Mdm), (doc, Source::Portal), (local, Source::Local)]
            .into_iter()
            .find_map(|(raw, source)| f(raw).clone().map(|v| (Some(v), source)))
            .unwrap_or((None, Source::Default))
    };
    let (endpoint, s_endpoint) = pick(|r| &r.endpoint);
    let (level, s_level) = pick(|r| &r.level);
    let (tools, s_tools) = pick(|r| &r.tools);
    let (org_name, s_org_name) = pick(|r| &r.org_name);
    let (web_access, s_web_access) = pick(|r| &r.web_access);
    let raw = Raw {
        endpoint,
        level,
        tools,
        org_name,
        web_access,
        update_notice: mdm.update_notice.clone().or_else(|| local.update_notice.clone()),
        managed: mdm.managed,
    };
    let sources =
        Sources { endpoint: s_endpoint, level: s_level, tools: s_tools, org_name: s_org_name, web_access: s_web_access };
    (raw, sources)
}

const UPDATE_LAUNCHER: &str = "Update the Consus Launcher to use your organization's settings.";

/// The org's document as the gateway serves it (its member view): the five
/// values the merge uses. The cache keeps the whole body as received, so a
/// newer launcher reads whatever this one skips. Null and absent both mean
/// "not set here".
#[derive(Default, Deserialize)]
struct Document {
    tools: Option<Value>,
    web_access: Option<Value>,
    level: Option<Value>,
    org_name: Option<Value>,
    endpoint: Option<Value>,
}

/// The document's values this launcher can use, ready for the merge, and
/// whether it carries a value this launcher version does not know, which
/// hides every tool (UPDATE_LAUNCHER). A field device management sets is not
/// looked at. The person cannot fix the document, so nothing here is the
/// "invalid settings" error; but nothing falls back to a default that would
/// loosen what the admin chose either: a bad endpoint keeps the one the
/// document was read from, and an unknown level or tool list hides the tools.
fn check_document(doc: &Document, fetched_from: &str, mdm: &Raw) -> (Raw, bool) {
    let mut raw = Raw::default();
    let mut outdated = false;
    // The document's value for a field device management left unset.
    let unset = |slot: &Option<Value>, value: &Option<Value>| slot.is_none().then(|| value.clone()).flatten();
    if let Some(v) = unset(&mdm.endpoint, &doc.endpoint) {
        let endpoint = endpoint_of(Some(v)).unwrap_or_else(|e| {
            eprintln!("device policy: endpoint not used ({e}); keeping {fetched_from}");
            fetched_from.to_string()
        });
        raw.endpoint = Some(Value::String(endpoint));
    }
    if let Some(v) = unset(&mdm.level, &doc.level) {
        match string(Some(v), "level", false) {
            Ok(Some(l)) if valid_level(&l) => raw.level = Some(Value::String(l)),
            _ => outdated = true,
        }
    }
    if let Some(v) = unset(&mdm.tools, &doc.tools) {
        // An empty list is "no tools", as from device management; a list
        // with no id this launcher knows names newer tools.
        match tool_ids(v.clone()) {
            Ok(ids) if ids.is_empty() || !tool_keys(&ids).is_empty() => raw.tools = Some(v),
            _ => outdated = true,
        }
    }
    if let Some(v) = unset(&mdm.org_name, &doc.org_name) {
        // Blank is not set, so it cannot blank out this computer's OrgName.
        match string(Some(v.clone()), "org_name", true) {
            Ok(Some(_)) => raw.org_name = Some(v),
            Ok(None) => {}
            Err(e) => eprintln!("device policy: org_name not used ({e})"),
        }
    }
    if let Some(v) = unset(&mdm.web_access, &doc.web_access) {
        match flag(Some(v.clone()), "web_access") {
            Ok(_) => raw.web_access = Some(v),
            Err(e) => eprintln!("device policy: web_access not used ({e})"),
        }
    }
    (raw, outdated)
}

/// The last document read, as the cache file holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cached {
    /// The `device_policy` object exactly as the gateway sent it.
    pub document: Value,
    pub etag: Option<String>,
    /// The endpoint it was read from, kept when the document's own endpoint
    /// cannot be used.
    pub fetched_from: String,
    /// Which key read it (`key_hash`), so another key never inherits it.
    pub key_hash: String,
}

const CACHE_FILE: &str = "device-policy.json";
static CACHE_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Where the cache lives: the launcher's own config folder. Called once at
/// startup, before anything reads the settings.
pub fn init(config_dir: PathBuf) {
    let _ = CACHE_PATH.set(config_dir.join(CACHE_FILE));
}

/// The first 16 hex characters of the key's SHA-256: enough to tell keys
/// apart, not enough to be the key.
pub fn key_hash(key: &str) -> String {
    Sha256::digest(key.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// The cache, if it was read with `key`. One read with another key, or that
/// cannot be read, is removed. With no key there is nothing to use it with,
/// but it stays: a keychain read can fail for a moment, and sign out removes
/// the file itself.
fn read_cache(path: &Path, key: Option<&str>) -> Option<Cached> {
    let key = key?;
    let text = fs::read_to_string(path).ok()?;
    let cached = serde_json::from_str::<Cached>(&text).ok().filter(|c| c.key_hash == key_hash(key));
    if cached.is_none() {
        let _ = fs::remove_file(path);
    }
    cached
}

/// A temp file renamed over the cache, so an interrupted write never leaves
/// half a document.
fn write_cache(path: &Path, cached: &Cached) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(cached).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text + "\n").and_then(|_| fs::rename(&tmp, path)).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// "… are invalid: …", naming who can fix it.
fn invalid(e: &str, managed: bool) -> String {
    if managed {
        format!("Your organization's launcher settings are invalid: {e} Contact your IT admin.")
    } else {
        format!("The launcher settings in your own preferences are invalid: {e}")
    }
}

/// Everything the settings are made from, and what they came to.
struct Loaded {
    mdm: Raw,
    local: Raw,
    /// Device management and this computer alone, as before the portal: its
    /// error stands whatever the document says.
    base: Result<Settings, String>,
    /// A settings file could not be read, so there is no endpoint either.
    unreadable: Option<String>,
    doc: Option<Cached>,
    settings: Result<Settings, String>,
    endpoint: Result<String, String>,
    /// The endpoint without the document, for a key it was not read with.
    base_endpoint: Result<String, String>,
    sources: Sources,
}

impl Loaded {
    fn new(read: Result<(Raw, Raw), (String, bool)>, doc: Option<Cached>) -> Loaded {
        let (mdm, local, unreadable) = match read {
            Ok((mdm, local)) => (mdm, local, None),
            Err((e, managed)) => (Raw::default(), Raw::default(), Some(invalid(&e, managed))),
        };
        let (raw, sources) = merge(&mdm, &Raw::default(), &local);
        let base_endpoint = match &unreadable {
            Some(e) => Err(e.clone()),
            None => endpoint_of(raw.endpoint.clone()).map_err(|e| invalid(&e, sources.endpoint == Source::Mdm)),
        };
        let base = match &unreadable {
            Some(e) => Err(e.clone()),
            None => parse(raw).map_err(|e| invalid(&e, mdm.managed)),
        };
        // `apply` decides the rest.
        let mut loaded = Loaded {
            mdm,
            local,
            base: base.clone(),
            unreadable,
            doc: None,
            settings: base,
            endpoint: base_endpoint.clone(),
            base_endpoint,
            sources: Sources::default(),
        };
        loaded.apply(doc);
        loaded
    }

    /// Re-decides every value with `doc` as the portal's document.
    fn apply(&mut self, doc: Option<Cached>) {
        let (doc_raw, outdated) = match &doc {
            Some(c) => {
                let document = serde_json::from_value::<Document>(c.document.clone()).unwrap_or_default();
                check_document(&document, &c.fetched_from, &self.mdm)
            }
            None => (Raw::default(), false),
        };
        let (raw, sources) = merge(&self.mdm, &doc_raw, &self.local);
        self.endpoint = match &self.unreadable {
            Some(e) => Err(e.clone()),
            None => endpoint_of(raw.endpoint.clone()).map_err(|e| invalid(&e, sources.endpoint == Source::Mdm)),
        };
        // The document's values were checked above, so parsing can only fail
        // where `base` already did.
        self.settings = match &self.base {
            Err(e) => Err(e.clone()),
            Ok(_) if outdated => Err(UPDATE_LAUNCHER.to_string()),
            Ok(_) => parse(raw).map_err(|e| invalid(&e, self.mdm.managed)),
        };
        self.sources = sources;
        self.doc = doc;
    }
}

static STATE: OnceLock<RwLock<Loaded>> = OnceLock::new();

/// The settings, read on first use: device management, this computer's
/// preferences, and the cached document if this key read it.
fn state() -> &'static RwLock<Loaded> {
    STATE.get_or_init(|| {
        let read = if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|h| read(Path::new(&h))).unwrap_or(Ok(Default::default()))
        } else {
            Ok(read_registry())
        };
        let key = crate::keychain::get_key();
        let doc = CACHE_PATH.get().and_then(|p| read_cache(p, key.as_deref()));
        RwLock::new(Loaded::new(read, doc))
    })
}

/// Reads the state. A panic while it was held leaves the last good state,
/// not a launcher that can no longer read its settings.
fn with<T>(f: impl FnOnce(&Loaded) -> T) -> T {
    f(&state().read().unwrap_or_else(PoisonError::into_inner))
}

/// Re-decides the settings with `doc` as the portal's document.
fn apply(doc: Option<Cached>) {
    state().write().unwrap_or_else(PoisonError::into_inner).apply(doc);
}

/// The settings now. Device management and this computer's preferences are
/// read on first use (restart the launcher to pick up a change there); the
/// portal's document changes them whenever a read brings a new one.
pub fn current() -> Result<Settings, String> {
    with(|l| l.settings.clone())
}

/// Where requests go, decided on its own: the launcher's own calls still have
/// a host when another value hides the tools. An invalid endpoint from device
/// management or this computer is still an error.
pub fn endpoint() -> Result<String, String> {
    with(|l| l.endpoint.clone())
}

/// Where requests made with `key` go. A document read with another key
/// (another org's, perhaps) never decides it: its endpoint may be that org's
/// own logging proxy.
pub fn endpoint_for(key: &str) -> Result<String, String> {
    with(|l| match &l.doc {
        Some(c) if c.key_hash != key_hash(key) => l.base_endpoint.clone(),
        _ => l.endpoint.clone(),
    })
}

/// Whether to say a newer launcher exists. From device management and this
/// computer only, so it still shows while the document asks for an update.
pub fn update_notice() -> bool {
    with(|l| l.base.as_ref().is_ok_and(|s| s.update_notice))
}

/// The portal's document as last read, if any.
pub fn cached() -> Option<Cached> {
    with(|l| l.doc.clone())
}

/// Saves a document just read and applies it. Whether anything changed.
pub fn store(c: Cached) -> bool {
    if cached().as_ref() == Some(&c) {
        return false;
    }
    if let Some(path) = CACHE_PATH.get() {
        if let Err(e) = write_cache(path, &c) {
            eprintln!("device policy: could not save ({e})");
        }
    }
    apply(Some(c));
    true
}

/// Drops the document, on disk and in use. Whether there was one.
pub fn forget() -> bool {
    if let Some(path) = CACHE_PATH.get() {
        let _ = fs::remove_file(path);
    }
    let had = cached().is_some();
    if had {
        apply(None);
    }
    had
}

/// Drops a document read with another key. Whether it did.
pub fn forget_other_key(key: &str) -> bool {
    cached().is_some_and(|c| c.key_hash != key_hash(key)) && forget()
}

/// The five values as the Keys tab lists them.
#[derive(Serialize)]
pub struct Values {
    pub endpoint: String,
    /// As people name it: "ITAR", "FedRAMP High + ITAR".
    pub level: String,
    /// Launcher keys of the tools shown; None is every tool.
    pub tools: Option<Vec<String>>,
    pub web_access: bool,
}

#[derive(Serialize)]
pub struct SettingsView {
    pub org_name: Option<String>,
    /// Why no tool is set up, complete with who can fix it.
    pub error: Option<String>,
    /// The line under the org name saying where the settings came from.
    pub header: Option<String>,
    pub sources: Sources,
    pub values: Option<Values>,
    /// When the org's admin last changed the portal's settings.
    pub portal_updated_at: Option<String>,
}

/// "Settings from your IT", "… the Consus portal", both, or nothing when
/// every value is this computer's or a default.
fn header(s: &Sources) -> Option<String> {
    match (s.any(Source::Mdm), s.any(Source::Portal)) {
        (true, true) => Some("Settings from your IT and the Consus portal".into()),
        (true, false) => Some("Settings from your IT".into()),
        (false, true) => Some("Settings from the Consus portal".into()),
        (false, false) => None,
    }
}

impl Loaded {
    fn view(&self) -> SettingsView {
        let ok = self.settings.as_ref().ok();
        SettingsView {
            org_name: ok.and_then(|s| s.org_name.clone()),
            error: self.settings.as_ref().err().cloned(),
            header: header(&self.sources),
            sources: self.sources.clone(),
            values: ok.map(|s| Values {
                endpoint: s.target.endpoint.clone(),
                level: s.target.tag(),
                tools: s.tools.clone(),
                web_access: s.target.web_access,
            }),
            portal_updated_at: self.doc.as_ref().and_then(|c| c.document.get("updated_at")?.as_str().map(String::from)),
        }
    }
}

/// Async so the first read (a few plutil runs) happens off the UI thread.
#[tauri::command]
pub async fn get_settings() -> SettingsView {
    with(Loaded::view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(endpoint: Option<Value>, level: Option<Value>, tools: Option<Value>) -> Raw {
        Raw { endpoint, level, tools, org_name: None, update_notice: None, web_access: None, managed: true }
    }

    /// The launcher as it would load with these sources and this document,
    /// read from `fetched_from`.
    fn loaded(mdm: Raw, local: Raw, doc: Option<Value>, fetched_from: &str) -> Loaded {
        let cached =
            doc.map(|document| Cached { document, etag: None, fetched_from: fetched_from.into(), key_hash: key_hash("k") });
        Loaded::new(Ok((mdm, local)), cached)
    }

    fn mdm(f: impl FnOnce(&mut Raw)) -> Raw {
        let mut r = Raw { managed: true, ..Raw::default() };
        f(&mut r);
        r
    }

    #[test]
    fn the_document_fills_what_device_management_left() {
        let l = loaded(
            mdm(|r| r.endpoint = Some(json!("https://proxy.mdm.example"))),
            Raw::default(),
            Some(json!({
                "endpoint": "https://proxy.portal.example",
                "level": "fedramp-high",
                "tools": ["claude-code", "pi"],
                "org_name": "Example Org",
                "web_access": true,
                "updated_at": "2026-11-11T18:04:11Z"
            })),
            "https://proxy.mdm.example",
        );
        let s = l.settings.clone().unwrap();
        assert_eq!(s.target.endpoint, "https://proxy.mdm.example");
        assert_eq!(l.endpoint.as_deref(), Ok("https://proxy.mdm.example"));
        assert_eq!(s.target.level, "fedramp-high");
        assert_eq!(s.tools.as_deref(), Some(&["code".to_string(), "pi".to_string()][..]));
        assert_eq!(s.org_name.as_deref(), Some("Example Org"));
        assert!(s.target.web_access);
        assert_eq!(
            l.sources,
            Sources {
                endpoint: Source::Mdm,
                level: Source::Portal,
                tools: Source::Portal,
                org_name: Source::Portal,
                web_access: Source::Portal
            }
        );
        let v = l.view();
        assert_eq!(v.header.as_deref(), Some("Settings from your IT and the Consus portal"));
        assert_eq!(v.portal_updated_at.as_deref(), Some("2026-11-11T18:04:11Z"));
        assert_eq!(v.values.map(|v| v.level), Some("FedRAMP High".into()));
    }

    #[test]
    fn device_management_wins_per_field() {
        let l = loaded(
            mdm(|r| {
                r.tools = Some(json!(["claude-code"]));
                r.level = Some(json!("il5"));
            }),
            Raw::default(),
            // The document's level is one this launcher does not know, but
            // device management sets the level, so it is never looked at.
            Some(json!({ "tools": ["pi"], "level": "il9" })),
            DEFAULT_ENDPOINT,
        );
        let s = l.settings.clone().unwrap();
        assert_eq!(s.tools.as_deref(), Some(&["code".to_string()][..]));
        assert_eq!(s.target.level, "il5");
        assert_eq!((l.sources.tools, l.sources.level), (Source::Mdm, Source::Mdm));
    }

    #[test]
    fn the_document_wins_over_this_computer() {
        let local = Raw { web_access: Some(json!("on")), org_name: Some(json!("Mine")), ..Raw::default() };
        let l = loaded(Raw::default(), local, Some(json!({ "web_access": false })), DEFAULT_ENDPOINT);
        let s = l.settings.clone().unwrap();
        assert!(!s.target.web_access);
        assert_eq!(s.org_name.as_deref(), Some("Mine"));
        assert_eq!((l.sources.web_access, l.sources.org_name), (Source::Portal, Source::Local));
        assert_eq!(l.view().header.as_deref(), Some("Settings from the Consus portal"));
    }

    #[test]
    fn an_unknown_document_level_is_an_error() {
        let l = loaded(
            Raw::default(),
            Raw::default(),
            Some(json!({ "level": "il9", "endpoint": "https://proxy.a.example" })),
            DEFAULT_ENDPOINT,
        );
        assert_eq!(l.settings, Err(UPDATE_LAUNCHER.to_string()), "shown as is, every tool hidden");
        assert_eq!(l.endpoint.as_deref(), Ok("https://proxy.a.example"), "the launcher's own calls still have a host");
        assert!(l.base.as_ref().is_ok_and(|s| s.update_notice), "the update link still shows");
    }

    #[test]
    fn a_bad_document_endpoint_keeps_the_one_the_read_used() {
        let l = loaded(
            Raw::default(),
            Raw::default(),
            Some(json!({ "endpoint": "http://proxy.example" })),
            "https://proxy.a.example",
        );
        assert_eq!(l.endpoint.as_deref(), Ok("https://proxy.a.example"));
        assert_eq!(l.settings.unwrap().target.endpoint, "https://proxy.a.example", "never the default");
    }

    #[test]
    fn an_unknown_document_tool_id_is_skipped() {
        let tools = |list: Value| loaded(Raw::default(), Raw::default(), Some(json!({ "tools": list })), DEFAULT_ENDPOINT).settings;
        assert_eq!(
            tools(json!(["claude-desktop", "future-tool"])).unwrap().tools.as_deref(),
            Some(&["desktop".to_string()][..])
        );
        assert_eq!(tools(json!(["future-tool"])), Err(UPDATE_LAUNCHER.to_string()), "not every tool, and not none");
        assert_eq!(tools(json!(42)), Err(UPDATE_LAUNCHER.to_string()));
        assert_eq!(tools(json!([])).unwrap().tools, Some(vec![]), "empty is no tools, not an outdated launcher");
    }

    #[test]
    fn a_blank_document_org_name_leaves_this_computers() {
        let local = Raw { org_name: Some(json!("Mine")), ..Raw::default() };
        let l = loaded(Raw::default(), local, Some(json!({ "org_name": "  " })), DEFAULT_ENDPOINT);
        assert_eq!(l.settings.unwrap().org_name.as_deref(), Some("Mine"));
        assert_eq!(l.sources.org_name, Source::Local);
    }

    #[test]
    fn another_keys_document_never_decides_where_a_key_goes() {
        let doc = Some(json!({ "endpoint": "https://proxy.org-a.example" }));
        let l = loaded(Raw::default(), Raw::default(), doc, DEFAULT_ENDPOINT);
        assert_eq!(l.endpoint.as_deref(), Ok("https://proxy.org-a.example"));
        assert_eq!(l.base_endpoint.as_deref(), Ok(DEFAULT_ENDPOINT), "what endpoint_for gives another key");
    }

    #[test]
    fn a_bad_document_org_name_is_dropped() {
        let l = loaded(
            Raw::default(),
            Raw::default(),
            Some(json!({ "org_name": 42, "web_access": "sometimes", "tools": ["pi"] })),
            DEFAULT_ENDPOINT,
        );
        let s = l.settings.unwrap();
        assert_eq!(s.org_name, None);
        assert!(!s.target.web_access, "an unusable web_access is off, never on");
        assert_eq!(s.tools.as_deref(), Some(&["pi".to_string()][..]));
    }

    #[test]
    fn managed_and_update_notice_follow_device_management_only() {
        let all = json!({ "tools": ["pi"], "level": "itar", "org_name": "X", "web_access": false });
        let s = loaded(Raw::default(), Raw::default(), Some(all), DEFAULT_ENDPOINT).settings.unwrap();
        assert!(s.update_notice, "a document alone does not make the machine managed");
        let (raw, _) = merge(&Raw::default(), &Raw { tools: Some(json!(["pi"])), ..Raw::default() }, &Raw::default());
        assert!(!raw.managed);
        let it = loaded(mdm(|r| r.org_name = Some(json!("IT"))), Raw::default(), None, DEFAULT_ENDPOINT);
        assert!(!it.settings.unwrap().update_notice, "on a managed machine IT pushes updates");
    }

    #[test]
    fn device_management_errors_keep_todays_wording() {
        let l = loaded(mdm(|r| r.level = Some(json!("cui"))), Raw::default(), Some(json!({ "tools": ["pi"] })), DEFAULT_ENDPOINT);
        let e = l.settings.unwrap_err();
        assert!(e.starts_with("Your organization's launcher settings are invalid: "), "{e}");
        assert!(e.ends_with(" Contact your IT admin."), "{e}");
        assert_eq!(l.endpoint.as_deref(), Ok(DEFAULT_ENDPOINT), "a bad level does not take the endpoint with it");
        let own = loaded(Raw::default(), Raw { level: Some(json!("cui")), ..Raw::default() }, None, DEFAULT_ENDPOINT);
        assert!(own.settings.unwrap_err().starts_with("The launcher settings in your own preferences are invalid: "));
        let unreadable = Loaded::new(Err(("x.plist could not be read.".into(), true)), None);
        assert!(unreadable.endpoint.is_err(), "no endpoint from settings that could not be read");
    }

    #[test]
    fn no_document_means_no_portal_sources() {
        let l = loaded(Raw::default(), Raw::default(), None, DEFAULT_ENDPOINT);
        assert_eq!(l.sources, Sources::default());
        assert_eq!(l.view().header, None);
        assert_eq!(l.settings.unwrap(), parse(Raw::default()).unwrap());
    }

    #[test]
    fn a_cache_for_another_key_is_removed() {
        let dir = std::env::temp_dir().join(format!("consus-launcher-policy-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join(CACHE_FILE);
        let c = Cached {
            document: json!({ "tools": ["pi"], "future_field": [1] }),
            etag: Some("\"abc\"".into()),
            fetched_from: DEFAULT_ENDPOINT.into(),
            key_hash: key_hash("key-a"),
        };
        write_cache(&path, &c).unwrap();
        assert_eq!(read_cache(&path, Some("key-a")), Some(c.clone()), "kept whole, unknown fields included");
        assert_eq!(read_cache(&path, Some("key-b")), None);
        assert!(!path.exists(), "another key's copy is deleted");
        write_cache(&path, &c).unwrap();
        assert_eq!(read_cache(&path, None), None, "no key, no copy");
        assert!(path.exists(), "but kept: the keychain read may fail for a moment");
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(read_cache(&path, Some("key-a")), None);
        assert!(!path.exists(), "an unreadable copy is deleted");
        assert_eq!(key_hash("key-a").len(), 16);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_set_means_defaults() {
        let s = parse(Raw::default()).unwrap();
        assert_eq!(s.target, Target::default());
        assert_eq!(s.tools, None);
        assert!(s.allows("desktop") && s.allows("pi"));
        assert!(s.update_notice, "an unmanaged machine is told about updates");
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

    #[test]
    fn web_access_is_off_unless_the_org_turns_it_on() {
        let with = |v: Option<Value>| parse(Raw { web_access: v, ..Raw::default() }).map(|s| s.target.web_access);
        assert_eq!(with(None), Ok(false), "ITAR work: nothing fetched from the machine unless the org says so");
        assert_eq!(with(Some(json!("on"))), Ok(true));
        assert_eq!(with(Some(json!("off"))), Ok(false));
        assert_eq!(with(Some(json!(true))), Ok(true));
        assert_eq!(with(Some(json!("0x1"))), Ok(true), "a registry REG_DWORD");
        assert!(with(Some(json!("allowlist"))).is_err(), "an unknown value is an error, not off");
    }

    #[test]
    fn update_notice_follows_the_setting_and_defaults_to_off_when_managed() {
        let with = |v: Option<Value>, managed: bool| {
            parse(Raw { update_notice: v, managed, ..Raw::default() }).map(|s| s.update_notice)
        };
        assert_eq!(with(None, false), Ok(true), "a launcher people installed themselves says so");
        assert_eq!(with(None, true), Ok(false), "on a managed machine IT pushes updates");
        assert_eq!(with(Some(json!(true)), true), Ok(true));
        assert_eq!(with(Some(json!("0x1")), true), Ok(true), "a registry REG_DWORD");
        assert_eq!(with(Some(json!("false")), false), Ok(false));
        assert!(with(Some(json!("sometimes")), false).is_err());
    }
}
