// What every tool template shares: where tools point, which compliance
// level they use, and how model ids are read.

use serde_json::Value;
use std::collections::BTreeMap;

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
    /// Whether a tool's agent may fetch web pages from this machine (the
    /// org's WebAccess setting). Off by default: on ITAR work a page the
    /// agent fetches, and the hostname it fetches it from, leave the
    /// boundary. Web search is never on: it is a server tool the gateway
    /// refuses, so each tool is told not to offer it.
    pub web_access: bool,
    /// The connectors the org turned on (its device policy's `connectors`),
    /// each already checked against the policy document.
    pub connectors: Vec<Connector>,
    /// The org's model order (its device policy's `models`), or None for
    /// each tool's own order.
    pub models: Option<ModelChoice>,
    /// The MCP servers the org wrote itself (`custom` connectors), each
    /// already checked (custom.rs).
    pub custom: Vec<CustomServer>,
}

/// One MCP server the org wrote itself: its name and the blob each Claude
/// tool gets, written as given.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomServer {
    pub name: String,
    pub claude_code: Option<Value>,
    pub claude_desktop: Option<Value>,
}

/// One connector the org turned on: its kind and the fields its slots take
/// (Slack: `client_id`).
#[derive(Clone, Debug, PartialEq)]
pub struct Connector {
    pub kind: String,
    pub fields: BTreeMap<String, String>,
}

/// Slack turned on with this client ID, for tests.
#[cfg(test)]
pub fn slack(client_id: &str) -> Connector {
    Connector { kind: "slack".into(), fields: BTreeMap::from([("client_id".to_string(), client_id.to_string())]) }
}

/// An org model order, for tests.
#[cfg(test)]
pub fn order(list: &[&str], default: Option<&str>) -> ModelChoice {
    ModelChoice { list: list.iter().map(|s| s.to_string()).collect(), default: default.map(str::to_string) }
}

/// The model-order cases this launcher and the portal both run.
#[cfg(test)]
pub const MODEL_CHOICE_CASES: &str = include_str!("../../policy/model-choice-cases.json");

impl Default for Target {
    fn default() -> Self {
        Target {
            endpoint: DEFAULT_ENDPOINT.into(),
            level: DEFAULT_LEVEL.into(),
            web_access: false,
            connectors: Vec::new(),
            models: None,
            custom: Vec::new(),
        }
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

/// The org's model order: base ids ("claude-sonnet-5-5", no prefix, no
/// level) in the order tools list them, and the one they start on. It
/// orders and picks; it never hides a model or grants one, since the key
/// already decides what a tool can list.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ModelChoice {
    pub list: Vec<String>,
    pub default: Option<String>,
}

impl ModelChoice {
    /// The document's `models` as this launcher reads it. Strings only in
    /// `list`, a repeat dropped, order kept; a `default` outside the list is
    /// no default; other keys are ignored. Not an object, or no list left,
    /// is an error the caller logs: the tools then list as before.
    pub fn read(v: &Value) -> Result<ModelChoice, String> {
        let list = v.get("list").and_then(Value::as_array).ok_or("not an object with a list")?;
        let mut ids: Vec<String> = Vec::new();
        for id in list.iter().filter_map(Value::as_str) {
            if !ids.iter().any(|x| x == id) {
                ids.push(id.to_string());
            }
        }
        if ids.is_empty() {
            return Err("the list is empty".into());
        }
        let default = v.get("default").and_then(Value::as_str).filter(|d| ids.iter().any(|x| x == d)).map(str::to_string);
        Ok(ModelChoice { list: ids, default })
    }

    fn position(&self, base: &str) -> Option<usize> {
        self.list.iter().position(|x| x == base)
    }
}

/// A model a tool can list, by the base id the org's order names.
pub trait HasBase {
    fn base(&self) -> &str;
}

impl HasBase for ClaudeModel {
    fn base(&self) -> &str {
        &self.base
    }
}

impl<T: HasBase + ?Sized> HasBase for &T {
    fn base(&self) -> &str {
        (**self).base()
    }
}

/// A bare id with its level: "gpt-5.6-sol:itar".
impl HasBase for String {
    fn base(&self) -> &str {
        base_id(self)
    }
}

/// A bare id with its level, and what goes with it (`at_level`'s rows).
impl<V> HasBase for (String, V) {
    fn base(&self) -> &str {
        base_id(&self.0)
    }
}

/// "claude-opus-5:itar" -> "claude-opus-5": the id without its level.
pub fn base_id(id: &str) -> &str {
    id.split_once(':').map_or(id, |(b, _)| b)
}

/// The candidates (a tool's models, in its own order) in the org's order:
/// the ones the list places, in list order, then every other one in the
/// tool's own order. Always every candidate, so the order never empties a
/// tool, and a list that places none of them leaves them as they are.
pub fn ordered<T: HasBase>(mut candidates: Vec<T>, choice: Option<&ModelChoice>) -> Vec<T> {
    if let Some(c) = choice {
        // Stable: unplaced ones keep the tool's order after the placed ones.
        candidates.sort_by_key(|m| c.position(m.base()).unwrap_or(usize::MAX));
    }
    candidates
}

/// The org's named default, when the tool has it.
pub fn named<'a, T: HasBase>(ordered: &'a [T], choice: Option<&ModelChoice>) -> Option<&'a T> {
    let d = choice?.default.as_deref()?;
    ordered.iter().find(|m| m.base() == d)
}

/// What a tool starts on by the org's choice: its default when the tool has
/// it, else the first of the order. None with no choice, or a list that
/// places none of the tool's models (a list naming only Claude models is no
/// choice for ChatGPT): the writer then keeps its own rule.
pub fn start<'a, T: HasBase>(ordered: &'a [T], choice: Option<&ModelChoice>) -> Option<&'a T> {
    let c = choice.filter(|c| ordered.iter().any(|m| c.position(m.base()).is_some()))?;
    named(ordered, Some(c)).or(ordered.first())
}

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
        // A zero is a limit the gateway does not know; treated as missing.
        context: num("context_window").filter(|n| *n > 0).unwrap_or(DEFAULT_CONTEXT),
        max_output: num("max_output_tokens").filter(|n| *n > 0).unwrap_or(DEFAULT_OUTPUT),
        image: row["input_modalities"].as_array().is_some_and(|a| a.iter().any(|m| m == "image")),
        efforts,
        pricing: [price("input"), price("output"), price("cache_read"), price("cache_write")],
        // The gateway reports no output cap for an embedding model.
        embedding: base.contains("embed") || row.get("max_output_tokens").is_some_and(Value::is_null),
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
        let t = |l: &str| Target { level: l.into(), ..Target::default() }.tag();
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
        assert!(detail(&serde_json::json!({ "id": "consus/some-vectors:itar", "max_output_tokens": null })).embedding);
        let zero = detail(&serde_json::json!({ "id": "consus/x:itar", "context_window": 0, "max_output_tokens": 0 }));
        assert_eq!((zero.context, zero.max_output), (128_000, 8_192));
        // An older gateway without the fields: safe defaults, named by id.
        let bare_row = detail(row(&m, "claude-opus-5-5:fedramp-high").unwrap());
        assert_eq!((bare_row.name.as_str(), bare_row.context, bare_row.max_output), ("claude-opus-5-5", 128_000, 8_192));
        assert_eq!(bare_row.efforts, ["low", "medium", "high"]);
    }

    struct Candidate(String);

    impl HasBase for Candidate {
        fn base(&self) -> &str {
            &self.0
        }
    }

    use super::order as choice;

    fn run(candidates: &[&str], c: Option<&ModelChoice>) -> (Vec<String>, Option<String>) {
        let list: Vec<Candidate> = candidates.iter().map(|b| Candidate(b.to_string())).collect();
        let order = ordered(list, c);
        let first = start(&order, c).map(|m| m.0.clone());
        (order.into_iter().map(|m| m.0).collect(), first)
    }

    const TOOL: [&str; 3] = ["claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5"];

    #[test]
    fn no_choice_changes_nothing() {
        assert_eq!(run(&TOOL, None), (TOOL.map(String::from).to_vec(), None));
    }

    #[test]
    fn the_list_orders_and_the_rest_follow() {
        let (order, _) = run(&TOOL, Some(&choice(&["claude-haiku-4-5", "claude-sonnet-5"], None)));
        assert_eq!(order, ["claude-haiku-4-5", "claude-sonnet-5", "claude-opus-5"]);
    }

    #[test]
    fn a_new_model_follows_the_ordered_ones() {
        let (order, _) = run(&["claude-opus-6", "claude-opus-5", "claude-sonnet-5"], Some(&choice(&["claude-sonnet-5", "claude-opus-5"], None)));
        assert_eq!(order, ["claude-sonnet-5", "claude-opus-5", "claude-opus-6"], "released after the admin saved: last, never hidden");
    }

    #[test]
    fn ids_the_key_lacks_are_skipped() {
        let (order, first) = run(&TOOL, Some(&choice(&["claude-opus-9", "claude-sonnet-5"], Some("claude-opus-9"))));
        assert_eq!(order, ["claude-sonnet-5", "claude-opus-5", "claude-haiku-4-5"]);
        assert_eq!(first.as_deref(), Some("claude-sonnet-5"), "a default the tool lacks starts the first of the order");
    }

    #[test]
    fn the_default_starts_only_when_the_tool_has_it() {
        let c = choice(&["claude-opus-5", "claude-sonnet-5"], Some("claude-sonnet-5"));
        assert_eq!(run(&TOOL, Some(&c)).1.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(run(&["claude-opus-5"], Some(&c)).1.as_deref(), Some("claude-opus-5"));
    }

    #[test]
    fn a_choice_without_a_default_starts_on_the_first_listed() {
        assert_eq!(run(&TOOL, Some(&choice(&["claude-sonnet-5"], None))).1.as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn no_choice_gives_no_start_model() {
        assert_eq!(run(&TOOL, None).1, None);
    }

    #[test]
    fn a_list_that_places_none_of_a_tools_models_keeps_its_start() {
        let c = choice(&["gpt-5.6-sol"], Some("gpt-5.6-sol"));
        assert_eq!(run(&TOOL, Some(&c)), run(&TOOL, None), "no choice, for this tool");
    }

    #[test]
    fn the_document_value_is_read_or_refused() {
        let read = |v: Value| ModelChoice::read(&v);
        let c = read(serde_json::json!({ "list": ["a", 7, "b", "a"], "default": "b", "show": true })).unwrap();
        assert_eq!(c, choice(&["a", "b"], Some("b")), "strings only, repeats dropped, other keys ignored");
        assert_eq!(read(serde_json::json!({ "list": ["a"], "default": "z" })).unwrap().default, None, "outside the list: no default");
        for bad in [serde_json::json!(null), serde_json::json!([]), serde_json::json!({}), serde_json::json!({ "list": [] }), serde_json::json!({ "list": [1] })] {
            assert!(read(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_shared_case_orders_and_starts() {
        let doc: Value = serde_json::from_str(MODEL_CHOICE_CASES).unwrap();
        for case in doc["cases"].as_array().unwrap() {
            let id = case["id"].as_str().unwrap();
            let candidates: Vec<&str> = case["candidates"].as_array().unwrap().iter().map(|c| c["base"].as_str().unwrap()).collect();
            let c = (!case["choice"].is_null()).then(|| ModelChoice::read(&case["choice"]).unwrap());
            let (order, first) = run(&candidates, c.as_ref());
            assert_eq!(serde_json::json!(order), case["expect"]["order"], "{id}");
            assert_eq!(serde_json::json!(first), case["expect"]["start"], "{id}");
        }
    }
}
