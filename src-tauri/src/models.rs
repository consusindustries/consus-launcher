// What every tool template shares: where tools point, which compliance
// level they use, and how model ids are read.

use serde_json::Value;

pub const DEFAULT_ENDPOINT: &str = "https://api.consus.io";
/// ITAR unless the org's settings say otherwise (decided 2026-09-22).
pub const DEFAULT_LEVEL: &str = "itar";

/// Where every tool sends its requests, and the compliance level every
/// model id carries. Comes from the org's settings (settings.rs).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// The Anthropic-surface root, no trailing slash: "https://api.consus.io",
    /// or the org's own logging proxy. The OpenAI surface is under /v1.
    pub endpoint: String,
    /// The id suffix: "itar", "fedramp-high", "fedramp-high+itar", ...
    pub level: String,
}

impl Default for Target {
    fn default() -> Self {
        Target { endpoint: DEFAULT_ENDPOINT.into(), level: DEFAULT_LEVEL.into() }
    }
}

impl Target {
    /// The OpenAI-compatible root: "<endpoint>/v1".
    pub fn v1(&self) -> String {
        format!("{}/v1", self.endpoint)
    }

    /// How people name the level: "ITAR", "FedRAMP High", "IL5 + ITAR".
    pub fn tag(&self) -> String {
        self.level.split('+').map(level_tag).collect::<Vec<_>>().join(" + ")
    }
}

fn level_tag(part: &str) -> String {
    match part {
        "itar" => "ITAR".into(),
        "fedramp-low" => "FedRAMP Low".into(),
        "fedramp-moderate" => "FedRAMP Moderate".into(),
        "fedramp-high" => "FedRAMP High".into(),
        other => other.to_uppercase(),
    }
}

/// A compliance level the gateway knows: FedRAMP low, moderate, or high, or
/// DoD IL2, IL4, or IL5, each optionally with +itar, or ITAR alone.
pub fn valid_level(level: &str) -> bool {
    const BASES: [&str; 6] = ["fedramp-low", "fedramp-moderate", "fedramp-high", "il2", "il4", "il5"];
    level == "itar" || BASES.contains(&level) || level.strip_suffix("+itar").is_some_and(|b| BASES.contains(&b))
}

const FAMILIES: [&str; 3] = ["opus", "sonnet", "haiku"];

/// "consus/claude-opus-5:itar" -> "claude-opus-5:itar", the form tools expect.
pub fn bare(id: &str) -> &str {
    id.strip_prefix("consus/").unwrap_or(id)
}

/// Every model id this key can use, bare.
pub fn ids(models_json: &Value) -> Vec<&str> {
    models_json
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .map(bare)
        .collect()
}

/// One Claude model at the target level, as a template sees it.
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
    pub fn label(&self, t: &Target) -> String {
        let mut fam = self.family.clone();
        fam.replace_range(..1, &self.family[..1].to_uppercase());
        let ver = self.version.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
        format!("{fam} {ver} {}", t.tag())
    }
}

// "consus/claude-opus-4-8:itar" -> ClaudeModel at that level; else None.
fn parse_claude(id: &str, level: &str) -> Option<ClaudeModel> {
    let bare = bare(id);
    let (base, suffix) = bare.split_once(':')?;
    if suffix != level {
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

/// The Claude models this key can use at the target level, opus then sonnet
/// then haiku, newest first within each family.
pub fn claude_models(models_json: &Value, t: &Target) -> Vec<ClaudeModel> {
    let mut out: Vec<ClaudeModel> = models_json
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .filter_map(|id| parse_claude(id, &t.level))
        .collect();
    let rank = |f: &str| FAMILIES.iter().position(|x| *x == f).unwrap_or(9);
    out.sort_by(|a, b| rank(&a.family).cmp(&rank(&b.family)).then(b.version.cmp(&a.version)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_tags() {
        for ok in ["itar", "fedramp-high", "fedramp-high+itar", "il5+itar", "il2"] {
            assert!(valid_level(ok), "{ok}");
        }
        for bad in ["", "ITAR", "itar+itar", "fedramp-high+cui", "il6", "cui"] {
            assert!(!valid_level(bad), "{bad}");
        }
        let t = |l: &str| Target { endpoint: DEFAULT_ENDPOINT.into(), level: l.into() }.tag();
        assert_eq!(t("itar"), "ITAR");
        assert_eq!(t("fedramp-high+itar"), "FedRAMP High + ITAR");
        assert_eq!(t("il5"), "IL5");
    }

    #[test]
    fn claude_models_follow_the_level() {
        let m = serde_json::json!([
            { "id": "consus/claude-opus-5:itar" },
            { "id": "consus/claude-opus-5:fedramp-high" },
            { "id": "consus/claude-sonnet-5:fedramp-high" }
        ]);
        let high = Target { level: "fedramp-high".into(), ..Target::default() };
        let ids: Vec<String> = claude_models(&m, &high).into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["claude-opus-5:fedramp-high", "claude-sonnet-5:fedramp-high"]);
        assert_eq!(claude_models(&m, &Target::default()).len(), 1);
    }
}
