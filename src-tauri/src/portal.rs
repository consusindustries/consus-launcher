use serde::Serialize;

const USER_AGENT: &str = concat!("ConsusLauncher/", env!("CARGO_PKG_VERSION"));

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

#[tauri::command]
pub async fn validate_key(key: String) -> Result<ConnectResult, ConnectError> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(network)?;

    let target = crate::settings::current()
        .map_err(|message| ConnectError::Rejected { message })?
        .target;
    let resp = client
        .get(format!("{}/models", target.v1()))
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

/// Where the portal says which launcher is newest: a static file anyone can
/// read, so the check sends no key and nothing about the user.
const LATEST_URL: &str = "https://portal.consus.io/assets/launcher/latest.json";
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
    if !crate::settings::current().ok()?.update_notice {
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
