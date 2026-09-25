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

/// What the gateway reports about one model (GET /v1/models, 2026-09-25):
/// display_name, context_window, max_output_tokens, input_modalities,
/// reasoning_efforts, and pricing in USD per million tokens. A gateway or
/// proxy that leaves a field out gets a conservative default, so a tool
/// still opens; a request that fits these limits works on any cloud behind
/// the id, since the gateway reports the smallest.
#[derive(Debug, PartialEq)]
pub struct Detail {
    /// "Claude Opus 5.5", without the level.
    pub name: String,
    pub maker: String,
    pub context: u64,
    pub max_output: u64,
    pub image: bool,
    /// Lowest first, as the gateway lists them. Empty: no reasoning.
    pub efforts: Vec<String>,
    /// input, output, cache read, cache write. Zero: not offered.
    pub pricing: [f64; 4],
    /// An embedding model, which no chat tool can use.
    pub embedding: bool,
}

const DEFAULT_CONTEXT: u64 = 128_000;
const DEFAULT_OUTPUT: u64 = 8_192;
const DEFAULT_EFFORTS: [&str; 3] = ["low", "medium", "high"];

/// The gateway's row for a bare id, if the key has it.
pub fn row<'a>(models_json: &'a Value, bare_id: &str) -> Option<&'a Value> {
    models_json
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(Value::as_str).map(bare) == Some(bare_id))
}

pub fn detail(row: &Value) -> Detail {
    let id = row.get("id").and_then(Value::as_str).map(bare).unwrap_or("");
    let base = id.split_once(':').map_or(id, |(b, _)| b);
    let num = |k: &str| row.get(k).and_then(Value::as_u64);
    let price = |k: &str| row["pricing"].get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let efforts = match row.get("reasoning_efforts").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        None => DEFAULT_EFFORTS.iter().map(|e| e.to_string()).collect(),
    };
    Detail {
        name: row.get("display_name").and_then(Value::as_str).unwrap_or(base).to_string(),
        maker: row.get("owned_by").and_then(Value::as_str).unwrap_or("").to_string(),
        context: num("context_window").unwrap_or(DEFAULT_CONTEXT),
        max_output: num("max_output_tokens").unwrap_or(DEFAULT_OUTPUT),
        image: row["input_modalities"].as_array().is_some_and(|a| a.iter().any(|m| m == "image")),
        efforts,
        pricing: [price("input"), price("output"), price("cache_read"), price("cache_write")],
        embedding: base.contains("embed"),
    }
}

/// Every chat model the key has at the target level, bare id and detail,
/// in the gateway's order.
pub fn at_level(models_json: &Value, t: &Target) -> Vec<(String, Detail)> {
    let suffix = format!(":{}", t.level);
    models_json
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = bare(m.get("id")?.as_str()?);
            id.ends_with(&suffix).then(|| (id.to_string(), detail(m)))
        })
        .filter(|(_, d)| !d.embedding)
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

    #[test]
    fn details_come_from_the_gateway() {
        let m = serde_json::json!([
            { "id": "consus/claude-opus-5-5:itar", "owned_by": "anthropic", "display_name": "Claude Opus 5.5",
              "context_window": 1000000, "max_output_tokens": 128000, "input_modalities": ["text", "image", "pdf"],
              "reasoning_efforts": ["low", "medium", "high", "xhigh"],
              "pricing": { "input": 4.8, "output": 24.0, "cache_read": 0.24, "cache_write": 6.0 } },
            { "id": "consus/gpt-4.1:itar", "display_name": "GPT-4.1", "input_modalities": ["text"], "reasoning_efforts": [] },
            { "id": "consus/titan-embed-text-v2:itar", "max_output_tokens": null },
            { "id": "consus/claude-opus-5-5:fedramp-high" }
        ]);
        let got = at_level(&m, &Target::default());
        let ids: Vec<&str> = got.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["claude-opus-5-5:itar", "gpt-4.1:itar"], "embeddings and other levels are left out");
        let opus = &got[0].1;
        assert_eq!((opus.name.as_str(), opus.context, opus.max_output, opus.image), ("Claude Opus 5.5", 1_000_000, 128_000, true));
        assert_eq!(opus.efforts, ["low", "medium", "high", "xhigh"]);
        assert_eq!(opus.pricing, [4.8, 24.0, 0.24, 6.0]);
        let gpt = &got[1].1;
        assert!(gpt.efforts.is_empty() && !gpt.image);
        // An older gateway without the fields: safe defaults, named by id.
        let bare_row = detail(row(&m, "claude-opus-5-5:fedramp-high").unwrap());
        assert_eq!((bare_row.name.as_str(), bare_row.context, bare_row.max_output), ("claude-opus-5-5", 128_000, 8_192));
        assert_eq!(bare_row.efforts, ["low", "medium", "high"]);
    }
}
