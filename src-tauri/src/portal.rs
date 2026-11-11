use serde::Serialize;
use serde_json::Value;
use std::sync::OnceLock;
use tauri::AppHandle;

use crate::settings::{self, Cached};

pub const USER_AGENT: &str = concat!("ConsusLauncher/", env!("CARGO_PKG_VERSION"));
pub const PORTAL_URL: &str = "https://portal.consus.io";

#[derive(Debug, Serialize)]
pub struct ConnectResult {
    pub models: serde_json::Value,
    pub model_count: usize,
    pub email: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind")]
pub enum ConnectError {
    Revoked,
    Network { message: String },
    Rejected { message: String },
}

/// The whole cause chain: reqwest's own message is only "error sending
/// request for url", and the reason (DNS, TLS, proxy) is in its sources.
fn network(e: impl std::error::Error) -> ConnectError {
    let mut message = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        message.push_str(": ");
        message.push_str(&c.to_string());
        cause = c.source();
    }
    ConnectError::Network { message }
}

/// One client for the gateway calls made with the key.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("a default TLS client")
    })
}

#[tauri::command]
pub async fn validate_key(key: String) -> Result<ConnectResult, ConnectError> {
    let endpoint = settings::endpoint_for(&key).map_err(|message| ConnectError::Rejected { message })?;
    let resp = client()
        .get(format!("{endpoint}/v1/models"))
        .header("x-api-key", key)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(network)?;

    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(ConnectError::Revoked);
        }
        return Err(ConnectError::Rejected {
            message: format!("portal returned {}", status),
        });
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(network)?;

    // The gateway returns an OpenAI-style listing: {"object":"list","data":[...]}
    let models = body
        .get("data")
        .cloned()
        .unwrap_or(serde_json::Value::Array(vec![]));
    let model_count = models.as_array().map(|a| a.len()).unwrap_or(0);
    let email = body
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(ConnectResult {
        models,
        model_count,
        email,
    })
}

/// What a read of the org's document came to.
#[derive(Debug, PartialEq)]
enum Fetched {
    /// The `device_policy` object and its ETag.
    Document(Value, Option<String>),
    /// The org has no document.
    None,
    /// 304: the cached copy is current.
    Unchanged,
}

/// The response, read. Anything but a document, null, or 304 is an error,
/// and every error keeps the cached copy: a 404 (a gateway without the route,
/// or a logging proxy that does not forward it), 401 and 403 (whether the key
/// is dead is `validate_key`'s to say), 402, and 5xx alike.
fn outcome(status: u16, etag: Option<String>, body: Option<&Value>) -> Result<Fetched, String> {
    match (status, body.and_then(|b| b.get("device_policy"))) {
        (304, _) => Ok(Fetched::Unchanged),
        (200, Some(Value::Null)) => Ok(Fetched::None),
        (200, Some(doc @ Value::Object(_))) => Ok(Fetched::Document(doc.clone(), etag)),
        (200, _) => Err("device policy: the gateway's answer has no device_policy object".into()),
        (s, _) => Err(format!("device policy: the gateway returned {s}")),
    }
}

/// GET /v1/me/device-policy with the key, sending the cached ETag.
async fn fetch_device_policy(key: &str, endpoint: &str, etag: Option<&str>) -> Result<Fetched, String> {
    let mut req = client()
        .get(format!("{endpoint}/v1/me/device-policy"))
        .header("x-api-key", key)
        .header("accept", "application/json");
    if let Some(etag) = etag {
        req = req.header("if-none-match", etag);
    }
    let resp = req.send().await.map_err(|e| format!("device policy: {e}"))?;
    let status = resp.status().as_u16();
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(String::from);
    let body = if status == 200 { resp.json::<Value>().await.ok() } else { None };
    outcome(status, etag, body.as_ref())
}

/// Reads the org's document with `key` and applies it. Whether the settings
/// changed, so the UI knows to reload. A tool the new settings turn off
/// loses the settings the launcher wrote for it, as at startup.
#[tauri::command]
pub async fn sync_device_policy(app: AppHandle, key: String) -> bool {
    let mut changed = settings::forget_other_key(&key);
    // No endpoint means device management's settings are unreadable or
    // invalid, which already hides every tool.
    if let Ok(endpoint) = settings::endpoint() {
        let etag = settings::cached().and_then(|c| c.etag);
        let fetched = fetch_device_policy(&key, &endpoint, etag.as_deref()).await;
        // The key may have been replaced, or signed out, while this read was
        // out; its answer is then not this machine's any more.
        if crate::keychain::get_key().as_deref() != Some(key.as_str()) {
            return changed;
        }
        match fetched {
            Ok(Fetched::Document(document, etag)) => {
                let key_hash = settings::key_hash(&key);
                changed |= settings::store(Cached { document, etag, fetched_from: endpoint, key_hash });
            }
            Ok(Fetched::None) => changed |= settings::forget(),
            Ok(Fetched::Unchanged) => {}
            Err(e) => eprintln!("{e}"),
        }
    }
    if changed {
        std::thread::spawn(move || crate::tools::remove_turned_off(&app));
    }
    changed
}

/// Where the portal says which launcher is newest: a static file anyone can
/// read, so the check sends no key and nothing about the user.
const LATEST_URL: &str = concat!("https://portal.consus.io", "/assets/launcher/latest.json");
/// The only place an update may point: this project's releases on GitHub.
const RELEASES_PATH: &str = "/consusindustries/consus-launcher/releases/";

#[derive(Debug, PartialEq, Serialize)]
pub struct Update {
    pub version: String,
    pub url: String,
}

/// "1.2.3" exactly: anything after the third number makes it no version.
fn semver(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let version = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(version)
}

/// The link as the browser would open it. Parsed, so dot segments ("..",
/// "%2e%2e") cannot walk out of this project's releases.
fn release_link(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url).ok()?;
    (u.scheme() == "https" && u.host_str() == Some("github.com") && u.path().starts_with(RELEASES_PATH))
        .then(|| u.as_str().to_string())
}

/// The update in `latest`, when it is newer than `current` and its link is
/// one of this project's releases; anything else is ignored.
fn newer(latest: &serde_json::Value, current: &str) -> Option<Update> {
    let version = latest.get("version")?.as_str()?.trim().trim_start_matches('v');
    let url = release_link(latest.get("url")?.as_str()?)?;
    (semver(version)? > semver(current)?).then(|| Update { version: version.to_string(), url })
}

/// A newer launcher, if the settings allow telling the user and the portal
/// lists one. Any failure (offline, a blocked host, a bad file) is silence.
#[tauri::command]
pub async fn check_update() -> Option<Update> {
    if !settings::update_notice() {
        return None;
    }
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let latest: serde_json::Value = client.get(LATEST_URL).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
    newer(&latest, env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_304_keeps_the_cache() {
        assert_eq!(outcome(304, None, None), Ok(Fetched::Unchanged));
    }

    #[test]
    fn a_null_document_removes_it() {
        let body = serde_json::json!({ "device_policy": null });
        assert_eq!(outcome(200, None, Some(&body)), Ok(Fetched::None));
    }

    #[test]
    fn a_200_writes_it_with_its_etag() {
        let body = serde_json::json!({ "device_policy": { "tools": ["pi"], "level": "itar" } });
        assert_eq!(
            outcome(200, Some("\"abc\"".into()), Some(&body)),
            Ok(Fetched::Document(serde_json::json!({ "tools": ["pi"], "level": "itar" }), Some("\"abc\"".into())))
        );
        assert!(outcome(200, None, Some(&serde_json::json!({ "other": 1 }))).is_err(), "no device_policy");
        assert!(outcome(200, None, None).is_err(), "no body");
    }

    #[test]
    fn a_401_or_403_keeps_the_cache_and_the_key() {
        // An error, not Revoked: the caller keeps the cache and the key.
        assert!(outcome(401, None, None).is_err());
        assert!(outcome(403, None, None).is_err());
    }

    #[test]
    fn a_404_keeps_the_cache() {
        assert!(outcome(404, None, None).is_err());
    }

    #[test]
    fn a_5xx_keeps_it() {
        for s in [402, 500, 502, 503] {
            assert!(outcome(s, None, None).is_err(), "{s}");
        }
    }

    #[test]
    fn an_update_is_newer_and_points_at_our_releases() {
        let latest = |v: &str, url: &str| serde_json::json!({ "version": v, "url": url });
        let ours = "https://github.com/consusindustries/consus-launcher/releases/tag/v0.3.0";
        assert_eq!(
            newer(&latest("0.3.0", ours), "0.2.4"),
            Some(Update { version: "0.3.0".into(), url: ours.into() })
        );
        assert_eq!(newer(&latest("v0.2.10", ours), "0.2.9").map(|u| u.version), Some("0.2.10".into()));
        assert_eq!(newer(&latest("0.2.4", ours), "0.2.4"), None, "same version");
        assert_eq!(newer(&latest("0.2.3", ours), "0.2.4"), None, "older");
        assert_eq!(newer(&latest("1.0", ours), "0.2.4"), None, "not a version");
        assert_eq!(newer(&latest("9.9.9", "https://evil.example/consus-launcher.dmg"), "0.2.4"), None, "not our releases");
        assert_eq!(newer(&serde_json::json!({ "version": "9.9.9" }), "0.2.4"), None, "no link");
        assert_eq!(newer(&latest("9.9.9.Install from evil.example", ours), "0.2.4"), None, "text after the version");
        for escape in [
            "https://github.com/consusindustries/consus-launcher/releases/../../../evil/repo/releases/download/v1/x.dmg",
            "https://github.com/consusindustries/consus-launcher/releases/%2e%2e/%2e%2e/%2e%2e/evil/repo/x.dmg",
            "http://github.com/consusindustries/consus-launcher/releases/tag/v9.9.9",
            "https://github.com.evil.example/consusindustries/consus-launcher/releases/tag/v9.9.9",
        ] {
            assert_eq!(newer(&latest("9.9.9", escape), "0.2.4"), None, "{escape}");
        }
    }
}
