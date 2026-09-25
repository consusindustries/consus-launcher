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

#[tauri::command]
pub async fn validate_key(key: String) -> Result<ConnectResult, ConnectError> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| ConnectError::Network { message: e.to_string() })?;

    let target = crate::settings::current()
        .map_err(|message| ConnectError::Rejected { message })?
        .target;
    let resp = client
        .get(format!("{}/models", target.v1()))
        .header("x-api-key", key)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| ConnectError::Network { message: e.to_string() })?;

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
        .map_err(|e| ConnectError::Network { message: e.to_string() })?;

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
