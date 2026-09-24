// What every tool template shares about model ids.

use serde_json::Value;

/// The Anthropic-surface root of the gateway (OpenAI-compatible under /v1).
pub const GATEWAY: &str = "https://api.consus.io";

// Fixed to ITAR for now by decision (2026-09-22); a regime rule is a later
// conversation, and this is the one place it will change.
pub const REGIME: &str = "itar";
pub const REGIME_TAG: &str = "ITAR";

const FAMILIES: [&str; 3] = ["opus", "sonnet", "haiku"];

/// "consus/claude-opus-5:itar" -> "claude-opus-5:itar", the form tools expect.
pub fn bare(id: &str) -> &str {
    id.strip_prefix("consus/").unwrap_or(id)
}

/// One Claude model in the regime, as a template sees it.
pub struct ClaudeModel {
    /// Bare id with suffix: "claude-opus-4-8:itar".
    pub id: String,
    /// Bare id without suffix: "claude-opus-4-8".
    pub base: String,
    pub family: String,
    pub version: Vec<u32>,
}

impl ClaudeModel {
    /// "Opus 4.8 ITAR", how a picker labels it.
    pub fn label(&self) -> String {
        let mut fam = self.family.clone();
        fam.replace_range(..1, &self.family[..1].to_uppercase());
        let ver = self.version.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
        format!("{fam} {ver} {REGIME_TAG}")
    }
}

// "consus/claude-opus-4-8:itar" -> ClaudeModel; anything else -> None.
fn parse_claude(id: &str) -> Option<ClaudeModel> {
    let bare = bare(id);
    let (base, suffix) = bare.split_once(':')?;
    if suffix != REGIME {
        return None;
    }
    let mut parts = base.strip_prefix("claude-")?.split('-');
    let family = parts.next()?.to_string();
    if !FAMILIES.contains(&family.as_str()) {
        return None;
    }
    let version: Vec<u32> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
    Some(ClaudeModel {
        id: bare.to_string(),
        base: base.to_string(),
        family,
        version,
    })
}

/// The Claude models this key can use in the regime, opus then sonnet then
/// haiku, newest first within each family.
pub fn claude_models(models_json: &Value) -> Vec<ClaudeModel> {
    let mut out: Vec<ClaudeModel> = models_json
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .filter_map(parse_claude)
        .collect();
    let rank = |f: &str| FAMILIES.iter().position(|x| *x == f).unwrap_or(9);
    out.sort_by(|a, b| rank(&a.family).cmp(&rank(&b.family)).then(b.version.cmp(&a.version)));
    out
}
