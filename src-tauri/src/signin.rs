// Sign in with the company account: the launcher side of the Consus CLI
// login protocol (gov_open_router specs/cli-login/README.md, protocol v1),
// the same one the credential helper script speaks.
//
// The launcher makes a one-time secret (the verifier), opens the portal's
// sign-in page with a hash of it (the challenge), and asks the portal's
// exchange endpoint, every couple of seconds, whether the person has
// approved. When they have, the portal mints a key owned by them, labelled
// with this computer, and hands it over once. The key goes into the OS
// keychain, as a pasted key does; a small signin.json next to the helper
// links records who signed in and when the key expires (no secret in it).
//
// A key the helper script already obtained on this machine (~/.consus/key)
// is adopted on first run rather than asking the person to sign in again.
// Sign out revokes the key through the gateway (DELETE /v1/me/key), which
// a sign-in key may do for itself; a pasted key may not, and the portal is
// where those are revoked.
//
// No sleeping here: each poll is one request, and the UI decides the pace.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

pub const PORTAL_URL: &str = "https://portal.consus.io";
/// How the portal names this tool on its approval page.
const TOOL: &str = "launcher";
const META_FILE: &str = "signin.json";
const USER_AGENT: &str = concat!("ConsusLauncher/", env!("CARGO_PKG_VERSION"));

/// The sign-in in progress: its verifier never leaves the Rust side.
pub struct Pending(pub Mutex<Option<String>>);

/// Who is signed in. Everything here is shown to the person; the key is
/// not in it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub org_id: String,
    #[serde(default)]
    pub key_id: String,
    #[serde(default)]
    pub label: String,
    /// "YYYY-MM-DD", or empty for a key that does not expire.
    #[serde(default)]
    pub expires: String,
    /// "signin" (this app), "helper" (adopted from ~/.consus), "pasted".
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind")]
pub enum SigninError {
    /// Nothing started, or it was cancelled.
    NotStarted,
    /// Approved more than ten minutes ago, or already used: start over.
    Expired,
    /// The portal could not issue a key; its reason, for the person.
    Refused { message: String },
    Network { message: String },
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE S256 over the verifier's ASCII text, as the helper and the portal do.
pub fn challenge_of(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

fn new_verifier() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness: {e}"))?;
    Ok(b64url(&bytes))
}

/// This computer's name as the portal accepts it: `[A-Za-z0-9 ._-]`, at most
/// 48 characters, never empty.
pub fn device_name(raw: &str) -> String {
    let s: String = raw
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || " ._-".contains(c) { c } else { '_' })
        .take(48)
        .collect();
    if s.is_empty() {
        "unknown".into()
    } else {
        s
    }
}

fn hostname() -> String {
    if cfg!(windows) {
        return device_name(&std::env::var("COMPUTERNAME").unwrap_or_default());
    }
    let out = std::process::Command::new("hostname").arg("-s").output().ok();
    device_name(&out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default())
}

pub fn login_url(challenge: &str, device: &str) -> String {
    format!("{PORTAL_URL}/cli/login?v=1&challenge={challenge}&device={}&tool={TOOL}", device.replace(' ', "%20"))
}

/// Where signin.json lives: next to the key-helper links.
fn meta_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join(META_FILE))
}

pub fn read_meta_at(path: &Path) -> Option<Meta> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn write_meta(app: &AppHandle, m: &Meta) -> Result<(), String> {
    let path = meta_path(app)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&path, serde_json::to_string_pretty(m).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))
}

/// Expired the day after its expiry date, as the gateway and the helper
/// count it. An unparseable or empty date never expires.
pub fn expired(expires: &str, today: &str) -> bool {
    let num = |s: &str| {
        let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        (d.len() == 8).then(|| d.parse::<u32>().ok()).flatten()
    };
    matches!((num(expires), num(today)), (Some(e), Some(t)) if t > e)
}

fn today() -> String {
    // Civil date from the Unix clock, UTC, as the helper's `date -u` gives.
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// What one exchange answer means (spec §2.3).
#[derive(Debug, PartialEq)]
pub enum Exchange {
    Pending,
    Key(Meta, String),
    Gone,
    Refused(String),
    BadRequest,
}

pub fn classify(status: u16, body: &Value) -> Exchange {
    match status {
        200 => match body.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()) {
            Some(key) => {
                let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                Exchange::Key(
                    Meta { owner: s("owner"), org_id: s("org_id"), key_id: s("key_id"), label: s("label"), expires: s("expires"), source: "signin".into() },
                    key.to_string(),
                )
            }
            None => Exchange::Refused("the portal returned no key; try again".into()),
        },
        410 => Exchange::Gone,
        400 => Exchange::BadRequest,
        // The approval was consumed but no key could be minted: final.
        502 if body.get("error").and_then(Value::as_str) == Some("mint_failed") => {
            let detail = body.get("detail").and_then(Value::as_str).unwrap_or("mint_failed");
            Exchange::Refused(format!("the portal could not issue a key ({detail}); sign in again"))
        }
        // 202 pending, 429 slow down, 5xx unavailable: keep asking.
        _ => Exchange::Pending,
    }
}

/// Starts a sign-in: remembers the verifier and returns the page to open in
/// the browser. A sign-in already in progress is replaced.
#[tauri::command]
pub fn start_signin(pending: State<'_, Pending>) -> Result<String, String> {
    let verifier = new_verifier()?;
    let url = login_url(&challenge_of(&verifier), &hostname());
    *pending.0.lock().unwrap() = Some(verifier);
    Ok(url)
}

#[tauri::command]
pub fn cancel_signin(pending: State<'_, Pending>) {
    *pending.0.lock().unwrap() = None;
}

/// One question to the portal: has the person approved? Ok(None) means not
/// yet (ask again in a couple of seconds). Ok(Some) means the key is in the
/// keychain and signin.json is written.
#[tauri::command]
pub async fn poll_signin(app: AppHandle, pending: State<'_, Pending>) -> Result<Option<Meta>, SigninError> {
    let Some(verifier) = pending.0.lock().unwrap().clone() else {
        return Err(SigninError::NotStarted);
    };
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| SigninError::Network { message: e.to_string() })?;
    let resp = client
        .post(format!("{PORTAL_URL}/api/cli/exchange"))
        .json(&serde_json::json!({ "verifier": verifier }))
        .send()
        .await
        .map_err(|e| SigninError::Network { message: e.to_string() })?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    match classify(status, &body) {
        Exchange::Pending => Ok(None),
        Exchange::Gone => {
            *pending.0.lock().unwrap() = None;
            Err(SigninError::Expired)
        }
        Exchange::BadRequest => {
            *pending.0.lock().unwrap() = None;
            Err(SigninError::Refused { message: "the portal rejected the sign-in request".into() })
        }
        Exchange::Refused(message) => {
            *pending.0.lock().unwrap() = None;
            Err(SigninError::Refused { message })
        }
        Exchange::Key(meta, key) => {
            *pending.0.lock().unwrap() = None;
            crate::keychain::set_key(&key).map_err(|message| SigninError::Refused { message })?;
            write_meta(&app, &meta).map_err(|message| SigninError::Refused { message })?;
            Ok(Some(meta))
        }
    }
}

/// Who is signed in, from signin.json; None when the key was pasted before
/// this file existed, or there is none.
#[tauri::command]
pub fn signin_info(app: AppHandle) -> Option<Meta> {
    read_meta_at(&meta_path(&app).ok()?)
}

/// Records a pasted key's provenance so the Keys tab can say so.
#[tauri::command]
pub fn note_pasted_key(app: AppHandle, owner: Option<String>) -> Result<(), String> {
    write_meta(&app, &Meta { owner: owner.unwrap_or_default(), source: "pasted".into(), ..Meta::default() })
}

/// The helper script's key on this machine (~/.consus/key with meta.json),
/// if there is one that has not expired.
pub fn helper_key(home: &Path, today: &str) -> Option<(String, Meta)> {
    let dir = home.join(".consus");
    let key = fs::read_to_string(dir.join("key")).ok()?.trim().to_string();
    if key.is_empty() {
        return None;
    }
    let m: Value = serde_json::from_str(&fs::read_to_string(dir.join("meta.json")).ok()?).ok()?;
    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let meta = Meta { owner: s("owner"), org_id: s("org_id"), key_id: s("key_id"), label: s("label"), expires: s("expires"), source: "helper".into() };
    (!expired(&meta.expires, today)).then_some((key, meta))
}

/// With no key in the keychain, adopts the helper script's key: into the
/// keychain, with its meta. Returns the key for the caller to validate.
#[tauri::command]
pub fn adopt_helper_key(app: AppHandle) -> Result<Option<String>, String> {
    if crate::keychain::get_key().is_some() {
        return Ok(None);
    }
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let Some((key, meta)) = helper_key(&home, &today()) else {
        return Ok(None);
    };
    crate::keychain::set_key(&key)?;
    write_meta(&app, &meta)?;
    Ok(Some(key))
}

#[derive(Debug, Serialize)]
pub struct Revoked {
    /// True: the key is dead everywhere. False: it can only be revoked in
    /// the portal (a pasted key), and the person is told so.
    pub revoked: bool,
    pub message: String,
}

/// Revokes the keychain's key through the gateway and forgets who was
/// signed in. The keychain entry and tool settings are the caller's to
/// remove next, whatever this answers.
#[tauri::command]
pub async fn revoke_key(app: AppHandle) -> Result<Revoked, String> {
    let _ = meta_path(&app).map(|p| fs::remove_file(p));
    let Some(key) = crate::keychain::get_key() else {
        return Ok(Revoked { revoked: true, message: "Signed out.".into() });
    };
    let target = crate::settings::current()?.target;
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.delete(format!("{}/me/key", target.v1())).header("x-api-key", key).send().await;
    Ok(match resp {
        Ok(r) if r.status().is_success() => Revoked { revoked: true, message: "Signed out. This computer's key is revoked.".into() },
        Ok(r) if r.status().as_u16() == 403 => {
            let body: Value = r.json().await.unwrap_or(Value::Null);
            // A bare 403 is a key already disabled; an expired key is dead too.
            let kind = body.pointer("/error/type").and_then(Value::as_str).unwrap_or("");
            if kind.is_empty() || kind == "key_expired" {
                Revoked { revoked: true, message: "Signed out. The key was already disabled.".into() }
            } else {
                Revoked { revoked: false, message: "Signed out on this computer. The key was pasted, so revoke it in the portal.".into() }
            }
        }
        Ok(r) => Revoked { revoked: false, message: format!("Signed out on this computer, but the gateway answered {}. Revoke the key in the portal.", r.status()) },
        Err(e) => Revoked { revoked: false, message: format!("Signed out on this computer, but the gateway could not be reached ({e}). Revoke the key in the portal.") },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_challenge_is_pkce_s256_over_the_verifier_text() {
        // RFC 7636 appendix B.
        assert_eq!(challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let v = new_verifier().unwrap();
        assert_eq!(v.len(), 43, "32 bytes, base64url, no padding");
        assert!(v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn the_device_name_fits_what_the_portal_accepts() {
        assert_eq!(device_name("eric-mbp\n"), "eric-mbp");
        assert_eq!(device_name("Eric's MacBook Air"), "Eric_s MacBook Air");
        assert_eq!(device_name(""), "unknown");
        assert_eq!(device_name(&"x".repeat(60)).len(), 48);
        assert_eq!(login_url("abc", "Eric MBP"), "https://portal.consus.io/cli/login?v=1&challenge=abc&device=Eric%20MBP&tool=launcher");
    }

    #[test]
    fn exchange_answers_are_read_as_the_spec_says() {
        assert_eq!(classify(202, &json!({ "status": "pending" })), Exchange::Pending);
        assert_eq!(classify(429, &Value::Null), Exchange::Pending);
        assert_eq!(classify(503, &json!({ "error": "cli_login_unavailable" })), Exchange::Pending);
        assert_eq!(classify(502, &json!({ "error": "x" })), Exchange::Pending, "an unavailable portal, not a refusal");
        assert_eq!(classify(410, &json!({ "error": "gone" })), Exchange::Gone);
        assert_eq!(classify(400, &json!({ "error": "bad_request" })), Exchange::BadRequest);
        assert!(matches!(classify(502, &json!({ "error": "mint_failed", "detail": "no models" })), Exchange::Refused(m) if m.contains("no models")));
        let ok = json!({ "key": "sk-1", "key_id": "k1", "expires": "2026-11-25", "org_id": "org", "owner": "e@x.gov", "label": "cli - mbp" });
        match classify(200, &ok) {
            Exchange::Key(m, k) => {
                assert_eq!(k, "sk-1");
                assert_eq!((m.owner.as_str(), m.expires.as_str(), m.source.as_str()), ("e@x.gov", "2026-11-25", "signin"));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(classify(200, &json!({})), Exchange::Refused(_)));
    }

    #[test]
    fn a_key_expires_the_day_after_its_date() {
        assert!(!expired("2026-11-25", "2026-11-25"));
        assert!(expired("2026-11-25", "2026-11-26"));
        assert!(!expired("", "2026-11-26"), "no date: never");
        assert!(!expired("soon", "2026-11-26"));
        let t = today();
        assert_eq!(t.len(), 10);
        assert!(t.starts_with("20"));
    }

    #[test]
    fn the_helper_scripts_key_is_adopted_unless_expired() {
        let home = std::env::temp_dir().join(format!("consus-launcher-signin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(".consus")).unwrap();
        assert!(helper_key(&home, "2026-10-26").is_none(), "nothing there");
        fs::write(home.join(".consus/key"), "sk-helper\n").unwrap();
        fs::write(home.join(".consus/meta.json"), r#"{"key_id":"k","expires":"2026-11-20","org_id":"o","owner":"e@x.gov","label":"cli - mbp"}"#).unwrap();
        let (k, m) = helper_key(&home, "2026-10-26").unwrap();
        assert_eq!((k.as_str(), m.owner.as_str(), m.source.as_str()), ("sk-helper", "e@x.gov", "helper"));
        assert!(helper_key(&home, "2026-11-21").is_none(), "expired the day after");
        let _ = fs::remove_dir_all(&home);
    }
}
