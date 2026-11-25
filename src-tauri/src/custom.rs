// Custom connectors: MCP servers an org chose itself, carried in its device
// policy as one JSON blob per client (docs/device-policy-models-and-custom-
// connectors.md): a remote server by its url, or for Claude Desktop one of
// its own servers by name (docs/connectors-registry-launcher.md). The launcher writes a blob as given, so it checks
// each one again first, with the gateway's rules: the document may come
// from an org's own proxy or a hand-edited cache, and the machine that would
// be harmed is the last check. policy/connector-cases.json holds the cases
// the gateway, the portal and this file all run.
//
// No process runs here and nothing here depends on the tools: this is the
// rule, the launcher's record of what it added, and which servers a person
// has been told about.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::models::CustomServer;

pub const CUSTOM_KIND: &str = "custom";
/// The two tools a custom server is written to, each by its own blob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    Code,
    Desktop,
}

impl Client {
    pub const ALL: [Client; 2] = [Client::Code, Client::Desktop];

    /// The blob's key in an entry.
    pub fn key(self) -> &'static str {
        match self {
            Client::Code => "claude_code",
            Client::Desktop => "claude_desktop",
        }
    }

    /// The remote transports this client's blob may name. `http` only in
    /// v1: proven by hand in both clients 2026-11-11. Another value is added
    /// only after a hand check, with the fixture.
    pub fn transports(self) -> &'static [&'static str] {
        match self {
            Client::Code | Client::Desktop => &["http"],
        }
    }

    /// The transport key this client's blob must carry. Claude Code reads a
    /// server without `type` as a command (stdio) server and refuses one
    /// that has only a URL. Claude Desktop's `transport` stays optional until
    /// a hand check shows otherwise.
    pub fn required_transport_key(self) -> Option<&'static str> {
        match self {
            Client::Code => Some("type"),
            Client::Desktop => None,
        }
    }
}
/// Claude Code reads `type`, Claude Desktop `transport`; both are checked in
/// both blobs, so a misplaced `stdio` is refused too.
pub const TRANSPORT_KEYS: [&str; 2] = ["type", "transport"];
/// Top-level keys that would run a program on the machine.
pub const COMMAND_KEYS: [&str; 4] = ["command", "args", "env", "headersHelper"];
/// Keys, at any depth, where a credential would go; matched without regard
/// to case, `_` or `-`.
pub const SECRET_KEYS: [&str; 2] = ["headers", "clientSecret"];
/// Claude Code replaces `${VAR}` with the person's environment variable.
pub const VARIABLE_MARKER: &str = "${";
/// A Claude Desktop blob with this key names one of Claude Desktop's own
/// servers (`"server": "microsoft365"`) and has no url. The launcher keeps no
/// list of them: which are offered is the portal's decision. Not the
/// "built-in" connectors of this code, which are the ones Consus defines in
/// the policy document (Slack).
pub const SERVER_KEY: &str = "server";
pub const BLOB_MAX_BYTES: usize = 4096;
pub const BLOB_MAX_DEPTH: usize = 8;
pub const CUSTOM_MAX: usize = 16;
pub const CONNECTORS_MAX: usize = 24;
pub const URL_MAX: usize = 256;

const NAME_MAX: usize = 64;

/// Why an entry is not used: the gateway's reason code, and a line that
/// names keys and the entry's name, never a value from a blob.
#[derive(Debug, PartialEq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

pub fn refuse(code: &'static str, message: impl Into<String>) -> Refusal {
    Refusal { code, message: message.into() }
}

/// The custom entries of one `connectors` list, checked in order: names are
/// taken as they are seen, so a later entry cannot take an earlier one's.
pub struct Check {
    /// Folded names a custom entry may not take: the built-in connectors'
    /// kinds, Claude Code keys and Claude Desktop names. Claude Desktop shows
    /// a custom server under its entry's name, so the entry's name rules keep
    /// Desktop's names apart too.
    built_in: Vec<String>,
    /// Folded names taken so far, the custom entries'.
    names: Vec<String>,
    seen: usize,
}

impl Check {
    /// A check against the built-in connectors in the policy document.
    pub fn new() -> Check {
        let p = crate::policy::policy();
        let fold = |s: &str| s.to_lowercase();
        let mut built_in: Vec<String> = p.connector_kinds().map(fold).collect();
        built_in.extend(p.connector_keys("claude-code").into_iter().map(fold));
        built_in.extend(p.connector_desktop_names().into_iter().map(fold));
        Check { built_in, names: Vec::new(), seen: 0 }
    }

    /// One `{ "kind": "custom", … }` entry: the server it names, or why not.
    /// First failure wins, in the gateway's order.
    pub fn entry(&mut self, entry: &Value) -> Result<CustomServer, Refusal> {
        self.seen += 1;
        if self.seen > CUSTOM_MAX {
            return Err(refuse("connectors_shape", format!("a custom connector past the first {CUSTOM_MAX}")));
        }
        let o = entry.as_object().ok_or_else(|| refuse("connector_field", "a custom connector that is not an object"))?;
        if o.keys().any(|k| k != "kind" && k != "name" && !Client::ALL.iter().any(|c| c.key() == k)) {
            return Err(refuse("connector_field", "a custom connector with a field this launcher does not know"));
        }
        // A name that can reach Claude Code is its key for the server and
        // goes on a command line: the strict rule. With no Claude Code part
        // the name is only what Claude Desktop shows, and may have spaces.
        let for_code = o.get(Client::Code.key()).is_some_and(|v| !v.is_null());
        let name = o.get("name").and_then(Value::as_str).filter(|n| valid_desktop_name(n));
        let name = name.ok_or_else(|| refuse("connector_name", "a custom connector without a valid name"))?;
        if for_code && !valid_name(name) {
            // Safe to print: it passed the wider rule. Without it the line
            // could not say which connector stopped being written.
            return Err(refuse("connector_name", format!("custom connector {name}: a name with a space cannot go to Claude Code")));
        }
        let folded = name.to_lowercase();
        if self.built_in.contains(&folded) {
            return Err(refuse("connector_name", format!("custom connector {name}: the name is a built-in connector's")));
        }
        if self.names.contains(&folded) {
            return Err(refuse("connector_name", format!("custom connector {name}: listed twice")));
        }
        self.names.push(folded);
        let present: Vec<(Client, &Value)> = Client::ALL.into_iter().filter_map(|c| Some((c, o.get(c.key()).filter(|v| !v.is_null())?))).collect();
        if present.is_empty() {
            return Err(refuse("connector_blob", format!("custom connector {name}: neither claude_code nor claude_desktop")));
        }
        for (client, blob) in &present {
            check_blob(blob, *client).map_err(|(code, why)| refuse(code, format!("custom connector {name} {}: {why}", client.key())))?;
        }
        let blob = |client: Client| present.iter().find(|(c, _)| *c == client).map(|(_, b)| (*b).clone());
        Ok(CustomServer { name: name.to_string(), claude_code: blob(Client::Code), claude_desktop: blob(Client::Desktop) })
    }
}

/// The gateway's name rule (MCP_SERVER_RE): 1-64 of letters, digits, `_`
/// and `-`. Safe on a command line and in a log line.
fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The name rule for an entry with no Claude Code part: words of the strict
/// rule's characters joined by one space, 64 at most with the spaces, so
/// "Microsoft 365" keeps the name a person signed in under. No space at
/// either end, no two in a row, no tab. Never a Claude Code key: Record::load
/// keeps the strict rule.
fn valid_desktop_name(name: &str) -> bool {
    name.len() <= NAME_MAX && name.split(' ').all(valid_name)
}

/// One client's blob, or the reason code and why not, naming keys only.
fn check_blob(blob: &Value, client: Client) -> Result<(), (&'static str, String)> {
    let o = blob.as_object().ok_or(("connector_blob", "not a JSON object".to_string()))?;
    // Compact, non-ASCII as is: the length the gateway measures.
    let size = serde_json::to_string(blob).map_or(usize::MAX, |s| s.len());
    if size > BLOB_MAX_BYTES {
        return Err(("connector_blob", format!("larger than {BLOB_MAX_BYTES} bytes")));
    }
    if depth(blob) > BLOB_MAX_DEPTH {
        return Err(("connector_blob", format!("nested more than {BLOB_MAX_DEPTH} levels")));
    }
    if let Some(k) = COMMAND_KEYS.iter().find(|k| o.contains_key(**k)) {
        return Err(("connector_command", format!("{k} would run a program")));
    }
    // One of Claude Desktop's own servers: no url to check, and Claude
    // Desktop drops one that also carries a url or a transport.
    if let Some(server) = o.get(SERVER_KEY) {
        let why = if client != Client::Desktop {
            Some(format!("{SERVER_KEY} is for Claude Desktop only"))
        } else if !server.as_str().is_some_and(valid_name) {
            Some(format!("{SERVER_KEY} is not a server name"))
        } else {
            (["url"].iter().chain(&TRANSPORT_KEYS)).find(|k| o.contains_key(**k)).map(|k| format!("{SERVER_KEY} with {k}"))
        };
        if let Some(why) = why {
            return Err(("connector_server", why));
        }
        return check_values(blob);
    }
    let allowed = client.transports();
    for key in TRANSPORT_KEYS {
        if o.get(key).is_some_and(|v| !v.as_str().is_some_and(|s| allowed.contains(&s))) {
            return Err(("connector_transport", format!("{key} is not one of {}", allowed.join(", "))));
        }
    }
    if let Some(key) = client.required_transport_key().filter(|k| !o.contains_key(*k)) {
        return Err(("connector_transport", format!("{key} is required")));
    }
    check_values(blob)?;
    if !o.get("url").and_then(Value::as_str).is_some_and(connector_url) {
        return Err(("connector_url", "url is missing or not a URL the gateway accepts".into()));
    }
    Ok(())
}

/// The checks every blob passes, with a url or without: no secret, no
/// variable reference.
fn check_values(blob: &Value) -> Result<(), (&'static str, String)> {
    if let Some(k) = SECRET_KEYS.iter().find(|k| has_key(blob, &fold_key(k))) {
        return Err(("connector_secret", format!("carries {k}")));
    }
    if has_marker(blob) {
        return Err(("connector_variable", format!("contains {VARIABLE_MARKER}")));
    }
    Ok(())
}

/// The blob is depth 1; each object or array inside it one more.
fn depth(v: &Value) -> usize {
    let inner = match v {
        Value::Object(m) => m.values().map(depth).max().unwrap_or(0),
        Value::Array(a) => a.iter().map(depth).max().unwrap_or(0),
        _ => return 0,
    };
    1 + inner
}

/// A key as the secret check compares it: lower case, no `_` or `-`.
fn fold_key(k: &str) -> String {
    k.chars().filter(|c| *c != '_' && *c != '-').flat_map(char::to_lowercase).collect()
}

fn has_key(v: &Value, folded: &str) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, v)| fold_key(k) == folded || has_key(v, folded)),
        Value::Array(a) => a.iter().any(|v| has_key(v, folded)),
        _ => false,
    }
}

fn has_marker(v: &Value) -> bool {
    match v {
        Value::String(s) => s.contains(VARIABLE_MARKER),
        Value::Object(m) => m.iter().any(|(k, v)| k.contains(VARIABLE_MARKER) || has_marker(v)),
        Value::Array(a) => a.iter().any(has_marker),
        _ => false,
    }
}

/// The gateway's URL rule for a custom server (its validate_device_endpoint,
/// which splits with Python's urlsplit): at most 256 characters, `https://`
/// (plain `http://` only to localhost or 127.0.0.1), a host of letters,
/// digits, `.` and `-`, no username or password, no query string or
/// fragment, a port of digits up to 65535. It only answers; the URL is
/// written exactly as given. Not `settings::normalize_endpoint`, which is
/// for the Consus endpoint and rewrites it. Stricter in one place: any
/// character outside printable ASCII is refused, where the gateway refuses
/// only what Python calls unprintable.
pub fn connector_url(url: &str) -> bool {
    if url.is_empty() || url.len() > URL_MAX || !url.bytes().all(|b| b.is_ascii_graphic()) {
        return false;
    }
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    let Some(rest) = rest.strip_prefix("//").filter(|_| scheme == "https" || scheme == "http") else {
        return false;
    };
    let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let netloc = rest.split('/').next().unwrap_or("");
    if !fragment.is_empty() || !query.is_empty() || netloc.contains('@') {
        return false;
    }
    let (host, port) = netloc.split_once(':').map_or((netloc, None), |(h, p)| (h, Some(p)));
    let host = host.to_ascii_lowercase();
    let host_ok = !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let port_ok = port.is_none_or(|p| p.is_empty() || (p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u32>().is_ok_and(|n| n <= 65535)));
    host_ok && port_ok && (scheme == "https" || host == "localhost" || host == "127.0.0.1")
}

/// The host of a checked server's URL, for the notice and the Keys tab:
/// "mcp.example.com".
pub fn host(url: &str) -> &str {
    let rest = url.split_once("//").map_or(url, |(_, r)| r);
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or("");
    netloc.split(':').next().unwrap_or(netloc)
}

/// What the launcher keeps about the org's own servers, in its own folder
/// (`connectors.json`). No value from a blob is in it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The names the launcher asked Claude Code to add, each with the hash
    /// (`blob_hash`) of the blob Claude Code was seen to hold from it, empty
    /// until then. With the built-in connectors' names and the names wanted
    /// now, the names it owns there: one removed in the portal is removed
    /// from Claude Code, and a server under any other name is never named.
    /// The hash is how a server counts as up to date: Claude Code keeps only
    /// the fields it knows (2.1.326 drops the rest), so what it holds never
    /// equals a blob with more.
    #[serde(default)]
    pub claude_code: BTreeMap<String, String>,
    /// Per server, the hash (`seen_hash`) of what the person was last told
    /// about.
    #[serde(default)]
    pub seen: BTreeMap<String, String>,
}

const RECORD_FILE: &str = "connectors.json";
static RECORD_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Where the record lives: the launcher's own config folder. Called once at
/// startup, beside settings::init.
pub fn init(config_dir: PathBuf) {
    let _ = RECORD_PATH.set(config_dir.join(RECORD_FILE));
}

pub fn record_path() -> Option<&'static Path> {
    RECORD_PATH.get().map(PathBuf::as_path)
}

impl Record {
    /// The record at `path`. Missing is empty; one that will not parse is
    /// empty too, logged, and replaced at the next save: losing it costs a
    /// notice shown again, and a stale name in Claude Code that the person
    /// can remove. A name that is not a valid server name is dropped: names
    /// from here go on a `claude mcp remove` command line, and on Windows an
    /// npm install runs that through cmd.exe, so a hand-edited file must not
    /// be able to put anything else there.
    pub fn load(path: &Path) -> Record {
        let mut r: Record = match fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                eprintln!("{}: {e}", path.display());
                Record::default()
            }),
            Err(_) => Record::default(),
        };
        r.claude_code.retain(|n, _| valid_name(n));
        r
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::settings::write_json_atomically(path, self)
    }
}

/// The launch's connector sync and the window's notices both change the
/// record; each change is a load, an edit and a save under this lock, so
/// neither saves over the other's.
static RECORD_LOCK: Mutex<()> = Mutex::new(());

/// Loads the record at `path`, lets `f` change it, and saves it if it
/// changed, under the record's lock.
pub fn update<T>(path: &Path, f: impl FnOnce(&mut Record) -> T) -> Result<T, String> {
    let _held = RECORD_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let mut r = Record::load(path);
    let before = r.clone();
    let out = f(&mut r);
    if r != before {
        r.save(path)?;
    }
    Ok(out)
}

/// At sign out, once Claude Code was asked to drop every recorded name:
/// the record goes when Claude Code holds none of them. When it may (Claude
/// Code missing, its state file busy), the names stay so the next run still
/// removes them, and what the person was told goes with the sign out.
pub fn sign_out(path: &Path, removed: bool) -> Result<(), String> {
    if !removed {
        return update(path, |r| r.seen.clear());
    }
    let _held = RECORD_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
        _ => Ok(()),
    }
}

/// One notice to show: on which tool's tile, and what it says.
#[derive(Debug, PartialEq)]
pub struct Notice {
    pub client: Client,
    pub message: String,
}

/// The notices due for `servers`: one per
/// server that is new or changed since the person was last told, on each
/// tile it is written to (`writable`: set up, not turned off by the org, and
/// not under the org's lock). A server the launcher writes nowhere right now
/// gets none and is not marked as told, so it is noticed once it is written.
/// It is for detection: the server is written at the next launch either way.
pub fn due(servers: &[CustomServer], writable: impl Fn(Client) -> bool, record: &mut Record) -> Vec<Notice> {
    let mut out = Vec::new();
    for s in servers {
        let clients: Vec<Client> = Client::ALL.into_iter().filter(|c| s.blob(*c).is_some() && writable(*c)).collect();
        let hash = seen_hash(s);
        let before = record.seen.get(&s.name);
        if clients.is_empty() || before == Some(&hash) {
            continue;
        }
        let verb = if before.is_some() { "changed" } else { "added" };
        let message = format!("Your organization {verb} the connector {} ({}).", s.name, s.shown());
        out.extend(clients.into_iter().map(|client| Notice { client, message: message.clone() }));
        record.seen.insert(s.name.clone(), hash);
    }
    // A server no longer in the document is new again if it comes back.
    record.seen.retain(|name, _| servers.iter().any(|s| &s.name == name));
    out
}

/// What a person was told about, as 12 hex characters: the SHA-256 of
/// `[name, claude_code, claude_desktop]` as compact JSON, object keys sorted
/// at every level by this code (not by the JSON library's map order), a
/// missing blob as null. A reordered or respaced document is no change; a
/// blob added to a server is.
pub fn seen_hash(s: &CustomServer) -> String {
    blob_hash(&json!([s.name, s.claude_code, s.claude_desktop]))
}

/// 12 hex characters of the SHA-256 of `v` as compact JSON, object keys
/// sorted at every level by this code, not by the JSON library's map order.
pub fn blob_hash(v: &Value) -> String {
    let mut text = String::new();
    canonical(v, &mut text);
    crate::settings::short_hex(&Sha256::digest(text.as_bytes()), 6)
}

fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

impl CustomServer {
    /// The server as a checked list holds it.
    pub fn to_value(&self) -> Value {
        let mut o = Map::new();
        o.insert("kind".into(), json!(CUSTOM_KIND));
        o.insert("name".into(), json!(self.name));
        for c in Client::ALL {
            if let Some(b) = self.blob(c) {
                o.insert(c.key().into(), b.clone());
            }
        }
        Value::Object(o)
    }

    /// From a checked list's entry. Only what `Check::entry` returned is in
    /// such a list.
    pub fn from_checked(v: &Value) -> Option<CustomServer> {
        let blob = |c: Client| v.get(c.key()).filter(|b| b.is_object()).cloned();
        Some(CustomServer { name: v.get("name")?.as_str()?.to_string(), claude_code: blob(Client::Code), claude_desktop: blob(Client::Desktop) })
    }

    /// The blob `client` gets.
    pub fn blob(&self, client: Client) -> Option<&Value> {
        match client {
            Client::Code => self.claude_code.as_ref(),
            Client::Desktop => self.claude_desktop.as_ref(),
        }
    }

    /// Its Claude Desktop entry as written: the blob as given, under the
    /// entry's name. A `name` in the blob is replaced, so the one name the
    /// admin gave the server is the one Claude Desktop shows, and the entry's
    /// name rules keep two servers from sharing one.
    pub fn desktop_entry(&self) -> Option<Value> {
        let mut blob = self.claude_desktop.clone()?;
        blob.as_object_mut()?.insert("name".into(), json!(self.name));
        Some(blob)
    }

    /// What the notice and the Keys tab show beside its name: the host its
    /// URL names, from whichever blob has one, else the Claude Desktop server
    /// it names. A checked blob has one or the other.
    pub fn shown(&self) -> &str {
        let from = |key: &str| Client::ALL.into_iter().filter_map(|c| self.blob(c)).find_map(|b| b.get(key)?.as_str());
        from("url").map(host).or_else(|| from(SERVER_KEY)).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../../policy/connector-cases.json");

    fn cases() -> Value {
        serde_json::from_str(CASES).unwrap()
    }

    #[test]
    fn the_fixture_header_equals_the_constants() {
        let doc = cases();
        let transports: Map<String, Value> = Client::ALL.iter().map(|c| (c.key().to_string(), json!(c.transports()))).collect();
        assert_eq!(doc["transports"], Value::Object(transports));
        assert_eq!(doc["transport_keys"], json!(TRANSPORT_KEYS));
        let required: Map<String, Value> =
            Client::ALL.iter().filter_map(|c| Some((c.key().to_string(), json!(c.required_transport_key()?)))).collect();
        assert_eq!(doc["required_transport_keys"], Value::Object(required));
        assert_eq!(doc["command_keys"], json!(COMMAND_KEYS));
        assert_eq!(doc["secret_keys"], json!(SECRET_KEYS));
        assert_eq!(doc["variable_marker"], VARIABLE_MARKER);
        assert_eq!(doc["blob_max_bytes"], BLOB_MAX_BYTES);
        assert_eq!(doc["blob_max_depth"], BLOB_MAX_DEPTH);
        assert_eq!(doc["custom_max"], CUSTOM_MAX);
        assert_eq!(doc["connectors_max"], CONNECTORS_MAX);
        assert_eq!(doc["url_max"], URL_MAX);
        assert_eq!(doc["server_key"], SERVER_KEY);
    }

    fn server(name: &str, desktop: Option<Value>) -> Value {
        let mut e = json!({ "kind": "custom", "name": name, "claude_code": { "type": "http", "url": "https://mcp.example.com/mcp" } });
        if let Some(d) = desktop {
            e["claude_desktop"] = d;
        }
        e
    }

    #[test]
    fn a_name_in_the_desktop_blob_is_allowed_and_not_checked() {
        let mut c = Check::new();
        let named = server("issues", Some(json!({ "name": "Slack", "transport": "http", "url": "https://mcp.example.com/mcp" })));
        assert!(c.entry(&named).is_ok(), "Claude Desktop shows the entry's name, so the blob's is never used");
        let odd = server("wiki", Some(json!({ "name": 7, "transport": "http", "url": "https://mcp.example.com/mcp" })));
        assert!(c.entry(&odd).is_ok());
    }

    #[test]
    fn a_url_ending_in_v1_is_written_as_given() {
        let mut c = Check::new();
        let e = json!({ "kind": "custom", "name": "issues", "claude_code": { "type": "http", "url": "https://mcp.example.com/api/v1/" } });
        let s = c.entry(&e).unwrap();
        assert_eq!(s.claude_code.unwrap()["url"], "https://mcp.example.com/api/v1/", "never rewritten");
    }

    #[test]
    fn urls_follow_the_gateways_rule() {
        for ok in ["https://mcp.example.com/mcp", "HTTPS://MCP.example.com", "http://localhost:8080/x", "http://LOCALHOST/x", "https://h:/x", "https://h:0/x", "https://h/x?"] {
            assert!(connector_url(ok), "{ok}");
        }
        for bad in ["", "mcp.example.com", "https:mcp.example.com", "ftp://h/x", "http://h.example.com/x", "https://u@h/x", "https://h/x?a=1", "https://h/x#f", "https://h_x/", "https://h:99999/", "https://h:8a/", "https://[::1]/", "https://h/é", "https://h/ x"] {
            assert!(!connector_url(bad), "{bad}");
        }
        assert_eq!(host("https://mcp.example.com:8443/mcp?x"), "mcp.example.com");
    }

    #[test]
    fn a_refusal_never_carries_a_value() {
        let mut c = Check::new();
        let e = json!({ "kind": "custom", "name": "issues", "claude_code": { "type": "http", "url": "https://h.example.com", "oauth": { "client_secret": "s3cr3t-value" } } });
        let r = c.entry(&e).unwrap_err();
        assert_eq!(r.code, "connector_secret");
        assert!(!r.message.contains("s3cr3t"), "{}", r.message);
    }

    fn issues(desktop: bool) -> CustomServer {
        CustomServer {
            name: "issues".into(),
            claude_code: Some(json!({ "type": "http", "url": "https://mcp.example.com/mcp", "oauth": { "clientId": "abc123", "callbackPort": 3119 } })),
            claude_desktop: desktop.then(|| json!({ "transport": "http", "url": "https://mcp.example.com/mcp" })),
        }
    }

    fn both(_: Client) -> bool {
        true
    }

    fn clients(n: &[Notice]) -> Vec<Client> {
        n.iter().map(|n| n.client).collect()
    }

    #[test]
    fn a_new_or_changed_server_is_noticed_once() {
        let mut r = Record::default();
        let n = due(&[issues(true)], both, &mut r);
        assert_eq!(clients(&n), [Client::Code, Client::Desktop], "on each tile it is written to");
        assert_eq!(n[0].message, "Your organization added the connector issues (mcp.example.com).");
        assert!(due(&[issues(true)], both, &mut r).is_empty(), "once");
        let mut moved = issues(true);
        moved.claude_code.as_mut().unwrap()["url"] = json!("https://mcp2.example.com/mcp");
        assert_eq!(due(&[moved], both, &mut r)[0].message, "Your organization changed the connector issues (mcp2.example.com).");
        due(&[], both, &mut r);
        assert!(r.seen.is_empty(), "removed: forgotten, so it is new if it comes back");
    }

    #[test]
    fn a_refused_server_is_never_noticed() {
        // Only what Check::entry returned reaches `due`; a refused entry is
        // not in Target.custom at all.
        let mut c = Check::new();
        let bad = json!({ "kind": "custom", "name": "issues", "claude_code": { "url": "https://h.example.com", "command": "npx" } });
        assert!(c.entry(&bad).is_err());
        let mut r = Record::default();
        assert!(due(&[], both, &mut r).is_empty() && r.seen.is_empty());
    }

    #[test]
    fn nothing_is_noticed_under_both_locks() {
        let mut r = Record::default();
        assert!(due(&[issues(true)], |_| false, &mut r).is_empty());
        assert!(r.seen.is_empty(), "not told, so not marked");
        assert_eq!(due(&[issues(true)], both, &mut r).len(), 2, "noticed once a lock is gone");
    }

    #[test]
    fn a_server_for_a_locked_tool_only_is_not_noticed() {
        let mut r = Record::default();
        let desktop_only = CustomServer { claude_code: None, ..issues(true) };
        assert!(due(&[desktop_only.clone()], |c| c == Client::Code, &mut r).is_empty(), "its only blob is for a locked tool");
        let n = due(&[desktop_only], |c| c == Client::Desktop, &mut r);
        assert_eq!(clients(&n), [Client::Desktop]);
    }

    #[test]
    fn reordered_keys_are_not_a_change() {
        let a = issues(false);
        let mut b = issues(false);
        b.claude_code = serde_json::from_str(r#"{ "oauth": { "callbackPort": 3119, "clientId": "abc123" }, "url": "https://mcp.example.com/mcp", "type": "http" }"#).ok();
        assert_eq!(seen_hash(&a), seen_hash(&b));
        assert_eq!(seen_hash(&a).len(), 12);
    }

    #[test]
    fn an_added_blob_is_a_change() {
        assert_ne!(seen_hash(&issues(false)), seen_hash(&issues(true)));
    }

    fn m365(desktop: Value) -> Value {
        json!({ "kind": "custom", "name": "Microsoft 365", "claude_desktop": desktop })
    }

    #[test]
    fn any_server_value_of_the_right_shape_is_kept() {
        // The shared fixture accepts only "microsoft365", because the portal
        // refuses a value it has no form for. The launcher keeps no list.
        let mut c = Check::new();
        let s = c.entry(&json!({ "kind": "custom", "name": "notes", "claude_desktop": { "server": "something_else-2" } })).unwrap();
        assert_eq!(s.shown(), "something_else-2");
    }

    #[test]
    fn a_refusal_for_a_bad_server_never_carries_it() {
        let mut c = Check::new();
        let r = c.entry(&m365(json!({ "server": "s3cr3t value" }))).unwrap_err();
        assert_eq!(r.code, "connector_server");
        assert!(!r.message.contains("s3cr3t"), "{}", r.message);
    }

    #[test]
    fn a_desktop_server_is_noticed_by_the_server_it_names() {
        let mut c = Check::new();
        let s = c.entry(&m365(json!({ "server": "microsoft365", "scope": "Mail.Read" }))).unwrap();
        let mut r = Record::default();
        let n = due(&[s.clone()], both, &mut r);
        assert_eq!(clients(&n), [Client::Desktop]);
        assert_eq!(n[0].message, "Your organization added the connector Microsoft 365 (microsoft365).", "never empty brackets");
        assert!(due(&[s.clone()], both, &mut r).is_empty(), "once");
        let mut wider = s;
        wider.claude_desktop.as_mut().unwrap()["scope"] = json!("Mail.Read Files.Read.All");
        assert_eq!(due(&[wider], both, &mut r)[0].message, "Your organization changed the connector Microsoft 365 (microsoft365).");
    }

    #[test]
    fn a_server_beside_a_remote_blob_shows_the_host() {
        let mut c = Check::new();
        let s = c.entry(&server("m365", Some(json!({ "server": "microsoft365" })))).unwrap();
        assert_eq!(s.shown(), "mcp.example.com", "the server it names only when neither blob has a url");
    }

    #[test]
    fn a_name_with_spaces_is_never_a_claude_code_key() {
        let mut c = Check::new();
        let r = c.entry(&server("my server", None)).unwrap_err();
        assert_eq!(r.code, "connector_name");
        assert!(r.message.contains("my server"), "named, so the log says which connector: {}", r.message);
        for bad in ["", " a", "a ", "a  b", "a\tb", "a\u{a0}b"] {
            assert!(!valid_desktop_name(bad), "{bad:?}");
        }
        assert!(valid_desktop_name("Microsoft 365") && !valid_name("Microsoft 365"));
    }

    #[test]
    fn a_record_name_that_is_not_a_server_name_is_dropped() {
        let dir = std::env::temp_dir().join(format!("consus-launcher-record-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("connectors.json");
        fs::write(&path, r#"{ "claude_code": { "issues": "", "x&calc": "", "a b": "" }, "seen": {} }"#).unwrap();
        assert_eq!(Record::load(&path).claude_code.into_keys().collect::<Vec<_>>(), ["issues"], "never on a command line");
        let _ = fs::remove_dir_all(&dir);
    }
}
