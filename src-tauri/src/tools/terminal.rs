// The terminal tools' per-tool part: what each one's Terminal window runs,
// and which inherited variables it must not see. Everything around it
// (finding, placing, tracking the window) stays in tools.rs.

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use super::*;
use crate::models::Target;
use crate::{claude_code, codex, config, factory, keychain, pi};

/// Unsets, in shell, every variable whose name matches the pattern, for a
/// Terminal window, whose environment the launcher cannot set directly.
fn scrub(names: &str) -> String {
    format!("for v in $(env | grep -oE \"^({names})=\" | tr -d =); do unset \"$v\"; done;")
}

/// Which variables each terminal tool must not inherit, as prefixes,
/// suffixes and exact names. The macOS shell scrub and the Windows
/// predicates (tools/win.rs) are both built from these, so they cannot
/// disagree.
pub(crate) struct Scrub {
    pub prefixes: &'static [&'static str],
    pub suffixes: &'static [&'static str],
    pub exact: &'static [&'static str],
}

impl Scrub {
    /// The grep -E pattern the shell scrub uses.
    fn pattern(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.prefixes.is_empty() {
            parts.push(format!("({})[A-Za-z0-9_]*", self.prefixes.join("|")));
        }
        parts.extend(self.suffixes.iter().map(|s| format!("[A-Za-z0-9_]*{s}")));
        parts.extend(self.exact.iter().map(|s| s.to_string()));
        parts.join("|")
    }

    pub fn matches(&self, name: &str) -> bool {
        self.prefixes.iter().any(|p| name.starts_with(p))
            || self.suffixes.iter().any(|s| name.ends_with(s))
            || self.exact.contains(&name)
    }
}

/// The same scrub as inherited_claude_var, for Claude Code.
pub(crate) const CLAUDE_SCRUB: Scrub = Scrub { prefixes: &["CLAUDE", "ANTHROPIC"], suffixes: &[], exact: &[] };
/// Anything that could point Codex at OpenAI directly or at another home.
pub(crate) const CODEX_SCRUB: Scrub = Scrub { prefixes: &["OPENAI", "CODEX"], suffixes: &[], exact: &[] };
/// Every credential Pi would otherwise use to offer another provider's
/// models next to Consus (its env-api-keys list: *_API_KEY, the cloud SDK
/// variables, HF_TOKEN, COPILOT_GITHUB_TOKEN), and the user's own
/// CONSUS_API_KEY, which Pi does not need and the agent's shell should not see.
pub(crate) const PI_SCRUB: Scrub = Scrub {
    prefixes: &["CLAUDE", "ANTHROPIC", "AWS", "GOOGLE", "GCLOUD", "AZURE"],
    suffixes: &["_API_KEY"],
    exact: &["HF_TOKEN", "COPILOT_GITHUB_TOKEN"],
};
/// Anything that could point droid at another home, org source, or
/// Factory's own key (FACTORY_API_KEY).
pub(crate) const FACTORY_SCRUB: Scrub = Scrub { prefixes: &["FACTORY"], suffixes: &[], exact: &[] };

/// What the version check found for the droid binary last checked. It holds
/// until that binary changes (an update). No answer is kept only for this run
/// of the launcher, so a network that blocks Factory costs one wait, not one
/// per launch, and a restart asks again.
static FACTORY_CHECKED: Mutex<Option<(PathBuf, Signature, factory::Check)>> = Mutex::new(None);

const FACTORY_REFUSED: &str = "This droid version ignores the launcher's Factory settings, so web search would be on. Ask IT for Factory's machine-wide lock, or use another tool.";
const FACTORY_UNSURE: &str = "Couldn't confirm that this droid version keeps web search off.";

/// Whether droid reads the launcher's org-level file, asked the way the
/// Terminal window will run it: same home, same file, same folder, droid's
/// folder first on PATH, and none of the FACTORY_ variables the launcher
/// inherited.
fn factory_check(cli: &Path, work: &Path, profile: &Path, org: &Path) -> factory::Check {
    let real = fs::canonicalize(cli).unwrap_or_else(|_| cli.to_path_buf());
    let sig = signature(&real);
    if let Some((p, s, c)) = FACTORY_CHECKED.lock().unwrap().as_ref() {
        if *p == real && Some(*s) == sig {
            return *c;
        }
    }
    let bin = cli.parent().unwrap_or(Path::new("/"));
    let mut cmd = Command::new(cli);
    cmd.args(["exec", "--list-tools", "--output-format", "json"]).current_dir(work);
    for (k, _) in std::env::vars_os() {
        if k.to_str().is_some_and(|k| FACTORY_SCRUB.matches(k)) {
            cmd.env_remove(k);
        }
    }
    cmd.env("FACTORY_HOME_OVERRIDE", profile)
        .env("FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH", org)
        .env("FACTORY_DROID_AUTO_UPDATE_ENABLED", "false")
        .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    // Measured under a second; the limit is for a network that holds the
    // connection to Factory open.
    let c = factory::check(output_within(&mut cmd, Duration::from_secs(10)).as_deref());
    if let Some(sig) = sig {
        *FACTORY_CHECKED.lock().unwrap() = Some((real, sig, c));
    }
    c
}

/// Factory's Terminal script. `org` is the launcher's org-level file, or
/// None under the organization's lock.
fn factory_script(work: &Path, profile: &Path, org: Option<&Path>, bin: &Path, cli: &Path) -> String {
    let org = org.map(|o| format!(" FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH=\"{}\"", o.display())).unwrap_or_default();
    format!(
        "cd \"{}\" || exit 1; {} export PATH=\"{}:$PATH\" FACTORY_HOME_OVERRIDE=\"{}\" FACTORY_DROID_AUTO_UPDATE_ENABLED=\"false\"{org}; clear; exec \"{}\"",
        work.display(),
        scrub(&FACTORY_SCRUB.pattern()),
        bin.display(),
        profile.display(),
        cli.display()
    )
}

/// Configures a terminal tool and returns the POSIX script its Terminal
/// window runs. The per-tool part; everything around it is shared.
pub(super) fn terminal_script(app: &AppHandle, t: Tool, models: &Value, target: &Target) -> Result<String, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let helper_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    // Both start in the same empty folder, so nothing of the user's is the
    // project: Claude Code would read ~/.claude as project settings from home.
    let work = claude_code::work_dir(&home);
    match t {
        Tool::Code => {
            let cli = cli(app, "claude").ok_or("Claude Code is not installed.")?;
            // Under the organization's policy the launcher writes nothing,
            // and removes the connectors it added before the lock came;
            // the profile folder still keeps the user's own ~/.claude apart.
            let locked = claude_code_policy();
            if !locked {
                let helper = helper_path(&helper_dir, CODE_HELPER_NAME);
                config::write_helper(&helper)?;
                claude_code::write_settings(&home, &helper, models, target)?;
            }
            launch_claude_connectors(app, &home, target, locked);
            fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
            // The user's own ~/.claude is never involved: an isolated profile
            // and a scrubbed environment.
            Ok(format!(
                "cd \"{}\" || exit 1; {} export CLAUDE_CONFIG_DIR=\"{}\"; clear; exec \"{}\"",
                work.display(),
                scrub(&CLAUDE_SCRUB.pattern()),
                claude_code::profile_dir(&home).display(),
                cli.display()
            ))
        }
        Tool::Codex => {
            let cli = codex_cli(app).ok_or("Codex is not installed.")?;
            keychain::get_key().ok_or("No key in the keychain. Connect first.")?;
            let helper = helper_path(&helper_dir, CODEX_HELPER_NAME);
            config::write_helper(&helper)?;
            let profile = codex::profile_dir(&home);
            fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
            // An npm install is a Node script; its node sits next to it, and
            // an app started from Finder has no node on its PATH.
            let bin = cli.parent().ok_or("Codex is not installed.")?;
            let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
            // The catalog is cloned from this Codex's own model list.
            let bundled = output_within(
                Command::new(&cli)
                    .args(["debug", "models", "--bundled"])
                    .env("CODEX_HOME", &profile)
                    .env("PATH", path),
                Duration::from_secs(10),
            )
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
            if bundled.is_none() {
                let _ = app.emit(
                    "tool-notice",
                    json!({ "tool": key(t), "message": "Codex could not list its models, so /model will not show the Consus ones this time." }),
                );
            }
            codex::write_config(&home, models, bundled.as_ref(), target)?;
            fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
            // Codex reads the key from CONSUS_API_KEY (env_http_headers), so
            // the script fetches it from the keychain helper into Codex's
            // environment only; the template's shell_environment_policy
            // keeps CONSUS_* out of every command the agent runs. No key, no
            // Codex: it would start and fail every request with no reason shown.
            Ok(format!(
                "cd \"{}\" || exit 1; {} CONSUS_API_KEY=\"$(\"{}\")\" || exit 1; export PATH=\"{}:$PATH\" CONSUS_API_KEY CODEX_HOME=\"{}\"; clear; exec \"{}\"",
                work.display(),
                scrub(&CODEX_SCRUB.pattern()),
                helper.display(),
                bin.display(),
                profile.display(),
                cli.display()
            ))
        }
        Tool::Pi => {
            let cli = cli(app, "pi").ok_or("Pi is not installed.")?;
            let helper = helper_path(&helper_dir, PI_HELPER_NAME);
            config::write_helper(&helper)?;
            pi::write_config(&home, &helper, models, target)?;
            fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
            // pi is a Node script; the node it was installed with (nvm, the
            // installer's own) sits next to it. The policy document's PI_
            // switches turn off Pi's calls to pi.dev (install telemetry,
            // update check). Not PI_OFFLINE: that also stops Pi fetching fd
            // and ripgrep, and its find and grep tools fail without them.
            let bin = cli.parent().ok_or("Pi is not installed.")?;
            // Values are quoted like the paths beside them; the document's
            // env values carry no quote or dollar (policy.rs checks).
            let pi_env: Vec<String> =
                crate::policy::policy().pi_env().into_iter().map(|(k, v)| format!("{k}=\"{v}\"")).collect();
            Ok(format!(
                "cd \"{}\" || exit 1; {} export PATH=\"{}:$PATH\" PI_CODING_AGENT_DIR=\"{}\" {}; clear; exec \"{}\"",
                work.display(),
                scrub(&PI_SCRUB.pattern()),
                bin.display(),
                pi::profile_dir(&home).display(),
                pi_env.join(" "),
                cli.display()
            ))
        }
        Tool::Factory => {
            let cli = cli(app, "droid").ok_or("Factory is not installed.")?;
            let bin = cli.parent().ok_or("Factory is not installed.")?;
            let profile = factory::profile_dir(&home);
            fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
            // Under the organization's lock the launcher writes nothing and
            // names no org-level file: the lock is the whole policy.
            let org = if factory_policy() {
                None
            } else {
                let helper = helper_path(&helper_dir, FACTORY_HELPER_NAME);
                config::write_helper(&helper)?;
                factory::write_config(&home, &helper, models, target)?;
                let org = factory::org_path(&home);
                match factory_check(&cli, &work, &profile, &org) {
                    factory::Check::Refuse => return Err(FACTORY_REFUSED.into()),
                    factory::Check::Unknown => {
                        let _ = app.emit("tool-notice", json!({ "tool": key(t), "message": FACTORY_UNSURE }));
                    }
                    factory::Check::Pass => {}
                }
                Some(org)
            };
            Ok(factory_script(&work, &profile, org.as_deref(), bin, &cli))
        }
        Tool::Desktop | Tool::ChatGpt => Err(format!("{} is not a terminal tool.", name(t))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_names_are_plain_identifiers_so_the_pattern_needs_no_escaping() {
        for s in [&CLAUDE_SCRUB, &CODEX_SCRUB, &PI_SCRUB, &FACTORY_SCRUB] {
            for n in s.prefixes.iter().chain(s.suffixes).chain(s.exact) {
                assert!(n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'), "{n}");
            }
        }
        assert!(inherited_claude_var("CLAUDE_CODE_X") && !inherited_claude_var("OPENAI_API_KEY"));
    }

    #[test]
    fn the_scrub_patterns_are_what_the_shell_always_ran_and_match_the_predicates() {
        assert_eq!(CLAUDE_SCRUB.pattern(), "(CLAUDE|ANTHROPIC)[A-Za-z0-9_]*");
        assert_eq!(CODEX_SCRUB.pattern(), "(OPENAI|CODEX)[A-Za-z0-9_]*");
        assert_eq!(
            PI_SCRUB.pattern(),
            "(CLAUDE|ANTHROPIC|AWS|GOOGLE|GCLOUD|AZURE)[A-Za-z0-9_]*|[A-Za-z0-9_]*_API_KEY|HF_TOKEN|COPILOT_GITHUB_TOKEN"
        );
        for (name, claude, codex, pi) in [
            ("CLAUDE_CONFIG_DIR", true, false, true),
            ("ANTHROPIC_API_KEY", true, false, true),
            ("OPENAI_API_KEY", false, true, true),
            ("CODEX_HOME", false, true, false),
            ("CONSUS_API_KEY", false, false, true),
            ("AWS_PROFILE", false, false, true),
            ("HF_TOKEN", false, false, true),
            ("PATH", false, false, false),
        ] {
            assert_eq!(CLAUDE_SCRUB.matches(name), claude, "{name} claude");
            assert_eq!(CODEX_SCRUB.matches(name), codex, "{name} codex");
            assert_eq!(PI_SCRUB.matches(name), pi, "{name} pi");
        }
    }

    #[test]
    fn factorys_script_names_its_home_and_org_file_and_never_the_key() {
        let p = |s: &str| PathBuf::from(s);
        let (work, profile, org) = (p("/Users/x/Consus"), p("/Users/x/.factory-consus-gateway"), p("/Users/x/.factory-consus-gateway/.factory/consus-org-settings.json"));
        let (bin, cli) = (p("/Users/x/.local/bin"), p("/Users/x/.local/bin/droid"));
        let s = factory_script(&work, &profile, Some(&org), &bin, &cli);
        assert!(s.starts_with("cd \"/Users/x/Consus\" || exit 1; for v in $(env | grep -oE \"^((FACTORY)[A-Za-z0-9_]*)=\""), "{s}");
        assert!(s.contains(" export PATH=\"/Users/x/.local/bin:$PATH\" FACTORY_HOME_OVERRIDE=\"/Users/x/.factory-consus-gateway\" FACTORY_DROID_AUTO_UPDATE_ENABLED=\"false\" FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH=\"/Users/x/.factory-consus-gateway/.factory/consus-org-settings.json\";"), "{s}");
        assert!(s.ends_with("; clear; exec \"/Users/x/.local/bin/droid\""), "{s}");
        assert!(!s.contains("KEY") && !s.contains('\''), "no key, and nothing Terminal's quoting cannot carry: {s}");
        let locked = factory_script(&work, &profile, None, &bin, &cli);
        assert!(locked.contains("FACTORY_HOME_OVERRIDE") && !locked.contains("FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH"), "under the lock: {locked}");
        assert!(FACTORY_SCRUB.matches("FACTORY_API_KEY") && FACTORY_SCRUB.matches("FACTORY_TELEMETRY_INGEST_BASE_URL") && !FACTORY_SCRUB.matches("PATH"));
    }

    /// Against the real droid on this machine (`droid` in ~/.local/bin):
    /// cargo test -- --ignored with_the_real_droid
    #[test]
    #[ignore]
    fn factory_check_with_the_real_droid() {
        let home = std::env::temp_dir().join(format!("consus-launcher-real-droid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let droid = PathBuf::from(std::env::var("HOME").unwrap()).join(".local/bin/droid");
        let work = home.join("Consus");
        fs::create_dir_all(&work).unwrap();
        let models = json!([{ "id": "consus/claude-opus-5-5:itar", "display_name": "Claude Opus 5.5" }]);
        factory::write_config(&home, Path::new("/usr/bin/true"), &models, &Target::default()).unwrap();
        let profile = factory::profile_dir(&home);
        assert_eq!(factory_check(&droid, &work, &profile, &factory::org_path(&home)), factory::Check::Pass, "with the launcher's org-level file");
        // Without the file, whether droid lists WebSearch depends on Factory's
        // side: it did on the morning of 2026-11-12 and did not that
        // afternoon, with no change here. So only check() pins the refusal,
        // on the output captured that morning.
        *FACTORY_CHECKED.lock().unwrap() = None;
        assert_ne!(factory_check(&droid, &work, &profile, &home.join("missing.json")), factory::Check::Unknown, "droid answers without the file too");
        let _ = fs::remove_dir_all(&home);
    }
}
