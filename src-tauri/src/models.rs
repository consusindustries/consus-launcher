// What every tool template shares about model ids.

// Fixed to ITAR for now by decision (2026-09-22); a regime rule is a later
// conversation, and this is the one place it will change.
pub const REGIME: &str = "itar";
pub const REGIME_TAG: &str = "ITAR";

/// "consus/claude-opus-5:itar" -> "claude-opus-5:itar", the form tools expect.
pub fn bare(id: &str) -> &str {
    id.strip_prefix("consus/").unwrap_or(id)
}
