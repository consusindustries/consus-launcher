// Claude Code's connectors: the `claude mcp` runs that make the servers in
// the launcher's profile what the org's settings say, at a launch, under a
// lock, and at sign out, and the record of the org's own servers the
// launcher added (custom.rs) kept in step with them.

use super::*;
use crate::custom;

/// The `claude` the launcher runs for `claude mcp`, ready for arguments.
#[cfg(windows)]
fn claude_command(_app: &AppHandle) -> Option<Command> {
    win::claude_command()
}

/// The `claude` the launcher runs for `claude mcp`, ready for arguments. An
/// npm install is a Node script; its node sits next to it, and an app
/// started from Finder has no node on its PATH.
#[cfg(not(windows))]
fn claude_command(app: &AppHandle) -> Option<Command> {
    let cli = cli(app, "claude")?;
    let mut c = Command::new(&cli);
    if let Some(bin) = cli.parent() {
        c.env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    }
    Some(c)
}

/// The `claude` for a run that carries values an admin typed (a custom
/// server's add): never through a shell. On Windows an npm install is a
/// `claude.cmd`, which runs through cmd.exe, where a `"` or `&` in a value
/// could end the quoting and run a command; there the script runs with
/// `node` directly (win.rs). Elsewhere `claude` already runs without one.
#[cfg(windows)]
fn claude_command_direct(_app: &AppHandle) -> Result<Command, String> {
    win::claude_command_direct()
}

#[cfg(not(windows))]
fn claude_command_direct(app: &AppHandle) -> Result<Command, String> {
    claude_command(app).ok_or_else(|| "Claude Code was not found.".to_string())
}

/// How `claude mcp` runs from the app: `true` for a run that must not go
/// through a shell.
fn claude_runs(app: &AppHandle) -> impl Fn(bool) -> Result<Command, String> + '_ {
    move |direct| if direct { claude_command_direct(app) } else { claude_command(app).ok_or_else(|| CONNECTORS_NOT_SET.to_string()) }
}

/// Where Claude Code's npm script and a `node.exe` to run it are, for the
/// `claude.cmd` npm put at `cmd`: the script beside it, and node beside it
/// or in a `path_dirs` folder. Full paths only, so Windows never looks for
/// a bare `node.exe` in the launcher's own folder. None when either is
/// missing: the server is then not added, with a notice.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn node_and_script(cmd: &Path, path_dirs: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    let dir = cmd.parent()?;
    let script = dir.join("node_modules").join("@anthropic-ai").join("claude-code").join("cli.js");
    if !script.is_file() {
        return None;
    }
    let node = std::iter::once(dir)
        .chain(path_dirs.iter().map(PathBuf::as_path))
        .filter(|d| d.is_absolute())
        .map(|d| d.join("node.exe"))
        .find(|n| n.is_file())?;
    Some((node, script))
}

const CONNECTORS_NOT_SET: &str =
    "Claude Code's connectors could not be updated this time, so one may be missing or still listed. Open Claude Code again to retry.";

/// The names the launcher owns in Claude Code: the built-in connectors',
/// those it recorded adding, and those wanted now.
fn owned_names(record: &custom::Record, wanted: &[(String, Value)]) -> Vec<String> {
    let mut owned: Vec<String> = crate::policy::policy().connector_keys("claude-code").into_iter().map(String::from).collect();
    for name in record.claude_code.iter().chain(wanted.iter().map(|(n, _)| n)) {
        if !owned.contains(name) {
            owned.push(name.clone());
        }
    }
    owned
}

/// Asks Claude Code to hold the servers `wanted` names under the launcher's
/// names in the launcher's profile: the org's connectors at a launch, none
/// under the org's lock or at sign out. `claude(direct)` builds each run.
/// Claude Code writes its own file; the launcher reads it before (nothing
/// to do, no run) and after (whether it worked). An error is for a notice,
/// never a failed launch.
pub(super) fn sync_claude_connectors(claude: &dyn Fn(bool) -> Result<Command, String>, home: &Path, wanted: &[(String, Value)]) -> Result<(), String> {
    // The session's own switches (no traffic to Anthropic), the launcher's
    // own Claude session scrubbed out, and the launcher's profile. Claude
    // Code reads the folder it runs in as a project (its .claude and
    // .mcp.json), so these runs start where sessions do: the empty ~/Consus,
    // never wherever the launcher was started from.
    let quiet = crate::policy::policy().claude_code_env_static();
    let (profile, work) = (claude_code::profile_dir(home), claude_code::work_dir(home));
    let mut ready = false;
    let mut run = |step: &claude_code::Step| -> Result<(), String> {
        if !ready {
            for dir in [&profile, &work] {
                fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            ready = true;
        }
        let mut cmd = claude(step.admin_text).map_err(|e| {
            if step.admin_text {
                format!("Claude Code's connector {} was not added: {e}", step.name)
            } else {
                e
            }
        })?;
        for (k, _) in std::env::vars_os() {
            if inherited_claude_var(&k.to_string_lossy().to_uppercase()) {
                cmd.env_remove(k);
            }
        }
        cmd.envs(quiet.iter().filter_map(|(k, v)| Some((k, v.as_str()?))));
        cmd.env("CLAUDE_CONFIG_DIR", &profile).current_dir(&work).args(&step.args);
        // "No server named" and "already exists" are fine answers; the file
        // says what happened.
        let _ = output_within(&mut cmd, Duration::from_secs(20));
        Ok(())
    };
    sync_steps(home, wanted, custom::record_path(), &mut run)
}

/// sync_claude_connectors with how a step runs left to `run`, and the
/// record at `record` (none in a run without one). The custom names about
/// to be added are recorded before any add runs, and a name is dropped once
/// Claude Code holds nothing under it and it is no longer wanted, so a crash
/// between the two costs one remove that finds nothing. Claude Code's state
/// file is read once when nothing needs to change, twice when something did.
fn sync_steps(
    home: &Path,
    wanted: &[(String, Value)],
    record: Option<&Path>,
    run: &mut dyn FnMut(&claude_code::Step) -> Result<(), String>,
) -> Result<(), String> {
    let rec = record.map(custom::Record::load).unwrap_or_default();
    let owned = owned_names(&rec, wanted);
    // Unreadable right now (a session midway through rewriting it): wait
    // for the next launch.
    let Some(plan) = claude_code::mcp_plan(home, wanted, &owned) else {
        return Ok(());
    };
    let mut failed: Vec<String> = Vec::new();
    let after = if plan.steps.is_empty() {
        Some(plan)
    } else {
        if let Some(path) = record {
            custom::update(path, |r| r.claude_code.extend(plan.steps.iter().filter(|s| s.admin_text).map(|s| s.name.clone())))?;
        }
        for step in &plan.steps {
            if let Err(e) = run(step) {
                if !failed.contains(&e) {
                    failed.push(e);
                }
            }
        }
        claude_code::mcp_plan(home, wanted, &owned)
    };
    if let (Some(after), Some(path)) = (&after, record) {
        custom::update(path, |r| r.claude_code.retain(|n| after.held.contains(n) || wanted.iter().any(|(w, _)| w == n)))?;
    }
    if !failed.is_empty() {
        return Err(failed.join(" "));
    }
    match after {
        Some(p) if !p.steps.is_empty() => Err(CONNECTORS_NOT_SET.to_string()),
        _ => Ok(()),
    }
}

/// sync_claude_connectors for a Claude Code launch: the org's connectors, or
/// none under the org's lock, which carries its own (and the launcher then
/// also takes back the deny rules it wrote before). A failure is a notice.
pub(super) fn launch_claude_connectors(app: &AppHandle, home: &Path, target: &Target, locked: bool) {
    let wanted = if locked { Vec::new() } else { claude_code::connector_servers(target) };
    let removed = if locked { claude_code::remove_connector_rules(home) } else { Ok(()) };
    if let Err(e) = removed.and(sync_claude_connectors(&claude_runs(app), home, &wanted)) {
        let _ = app.emit("tool-notice", json!({ "tool": key(Tool::Code), "message": e }));
    }
}

/// Claude Code turned off by the org: the launcher's servers go.
pub(super) fn remove_claude_connectors(app: &AppHandle, home: &Path) -> Result<(), String> {
    sync_claude_connectors(&claude_runs(app), home, &[])
}

/// At sign out: every server the launcher added goes, and then its record
/// (custom::sign_out keeps the names unless Claude Code is seen to hold none
/// of them, so a state file that could not be read leaves nothing behind
/// that the launcher would forget).
pub(super) fn sign_out_connectors(app: &AppHandle, home: &Path) -> Result<(), String> {
    let removed = remove_claude_connectors(app, home);
    let recorded = custom::record_path().map_or(Ok(()), |p| custom::sign_out(p, removed.is_ok() && none_held(home, p)));
    removed.and(recorded)
}

/// Whether Claude Code's state file can be read and holds none of the names
/// in the record at `record`.
fn none_held(home: &Path, record: &Path) -> bool {
    let names = custom::Record::load(record).claude_code;
    claude_code::mcp_plan(home, &[], &[]).is_some_and(|p| !p.held.iter().any(|h| names.contains(h)))
}

/// The one-time notices for the org's own servers (custom::due), as
/// `{ tool, message }` for the tiles. `set_up` is the tool keys the UI shows
/// as the launcher's to set up: allowed, installed, and not managed by the
/// org's lock (detect_tools has just worked that out).
pub(super) fn connector_notices(set_up: &[String]) -> Vec<Value> {
    let (Ok(s), Some(path)) = (settings::current(), custom::record_path()) else {
        return Vec::new();
    };
    let tool = |c: custom::Client| match c {
        custom::Client::Code => Tool::Code,
        custom::Client::Desktop => Tool::Desktop,
    };
    let writable = |c| set_up.iter().any(|k| k == key(tool(c)));
    let due = custom::update(path, |r| {
        if s.target.custom.is_empty() && r.seen.is_empty() {
            return Vec::new();
        }
        custom::due(&s.target.custom, writable, r)
    });
    let due = due.unwrap_or_else(|e| {
        eprintln!("connector notices: {e}");
        Vec::new()
    });
    due.into_iter().map(|n| json!({ "tool": key(tool(n.client)), "message": n.message })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consus-launcher-conn-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(claude_code::profile_dir(&dir)).unwrap();
        dir
    }

    fn state_file(home: &Path) -> PathBuf {
        claude_code::profile_dir(home).join(".claude.json")
    }

    fn held(home: &Path) -> Vec<String> {
        claude_code::mcp_plan(home, &[], &[]).unwrap().held
    }

    /// Claude Code as the steps see it: adds and removes in its state file.
    fn fake_claude(home: &Path) -> impl FnMut(&claude_code::Step) -> Result<(), String> + '_ {
        move |step| {
            let path = state_file(home);
            let mut state = claude_code::read_object(&path).unwrap();
            let servers = state.entry("mcpServers").or_insert_with(|| json!({})).as_object_mut().unwrap();
            if step.args[1] != "remove" {
                let entry = if step.args[1] == "add-json" {
                    serde_json::from_str(&step.args[3]).unwrap()
                } else {
                    json!({ "type": step.args[3], "url": step.args[5], "oauth": { "clientId": step.args[7], "callbackPort": step.args[9].parse::<u64>().unwrap() } })
                };
                servers.insert(step.name.clone(), entry);
            } else {
                servers.remove(&step.name);
            }
            fs::write(&path, Value::Object(state).to_string()).unwrap();
            Ok(())
        }
    }

    fn custom_server(name: &str) -> (String, Value) {
        (name.to_string(), json!({ "type": "http", "url": format!("https://{name}.example.com/mcp") }))
    }

    #[test]
    fn a_custom_server_is_added_and_recorded() {
        let home = temp_home("added");
        let record = home.join("connectors.json");
        let wanted = [custom_server("issues")];
        sync_steps(&home, &wanted, Some(&record), &mut fake_claude(&home)).unwrap();
        assert_eq!(held(&home), ["issues"]);
        assert_eq!(custom::Record::load(&record).claude_code, BTreeSet::from(["issues".to_string()]));
        let raw = fs::read_to_string(&record).unwrap();
        assert!(!raw.contains("example.com"), "no value from a blob in the record: {raw}");
        let mut none = |_: &claude_code::Step| -> Result<(), String> { panic!("already held: no run") };
        sync_steps(&home, &wanted, Some(&record), &mut none).unwrap();
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn removed_in_the_portal_it_is_removed_here() {
        let home = temp_home("removed");
        let record = home.join("connectors.json");
        sync_steps(&home, &[custom_server("issues"), custom_server("wiki")], Some(&record), &mut fake_claude(&home)).unwrap();
        sync_steps(&home, &[custom_server("wiki")], Some(&record), &mut fake_claude(&home)).unwrap();
        assert_eq!(held(&home), ["wiki"]);
        assert_eq!(custom::Record::load(&record).claude_code, BTreeSet::from(["wiki".to_string()]), "dropped once it is gone");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_persons_own_server_is_never_named() {
        let home = temp_home("theirs");
        let record = home.join("connectors.json");
        fs::write(state_file(&home), json!({ "mcpServers": { "mine": { "type": "stdio", "command": "x" } } }).to_string()).unwrap();
        let mut named: Vec<String> = Vec::new();
        let mut fake = fake_claude(&home);
        let mut run = |s: &claude_code::Step| {
            named.push(s.name.clone());
            fake(s)
        };
        sync_steps(&home, &[custom_server("issues")], Some(&record), &mut run).unwrap();
        sync_steps(&home, &[], Some(&record), &mut run).unwrap();
        assert_eq!(named, ["issues", "issues"]);
        assert_eq!(held(&home), ["mine"]);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn under_the_lock_recorded_names_go_and_none_are_added() {
        let home = temp_home("lock");
        let record = home.join("connectors.json");
        sync_steps(&home, &[custom_server("issues")], Some(&record), &mut fake_claude(&home)).unwrap();
        // Under the lock nothing is wanted (launch_claude_connectors).
        sync_steps(&home, &[], Some(&record), &mut fake_claude(&home)).unwrap();
        assert!(held(&home).is_empty());
        assert!(custom::Record::load(&record).claude_code.is_empty());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn sign_out_removes_recorded_names_and_the_record() {
        let home = temp_home("signout");
        let record = home.join("connectors.json");
        sync_steps(&home, &[custom_server("issues")], Some(&record), &mut fake_claude(&home)).unwrap();
        let removed = sync_steps(&home, &[], Some(&record), &mut fake_claude(&home));
        custom::sign_out(&record, removed.is_ok()).unwrap();
        assert!(held(&home).is_empty() && !record.exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_failed_sign_out_keeps_the_names() {
        let home = temp_home("signout-failed");
        let record = home.join("connectors.json");
        sync_steps(&home, &[custom_server("issues")], Some(&record), &mut fake_claude(&home)).unwrap();
        let mut r = custom::Record::load(&record);
        r.seen.insert("issues".into(), "3f9a1c0b7d2e".into());
        r.save(&record).unwrap();
        let mut missing = |_: &claude_code::Step| -> Result<(), String> { Err(CONNECTORS_NOT_SET.into()) };
        let removed = sync_steps(&home, &[], Some(&record), &mut missing);
        assert!(removed.is_err());
        custom::sign_out(&record, removed.is_ok()).unwrap();
        let left = custom::Record::load(&record);
        assert_eq!(left.claude_code, BTreeSet::from(["issues".to_string()]), "the next run still removes it");
        assert!(left.seen.is_empty(), "what the person was told goes with the sign out");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unreadable_state_file_at_sign_out_keeps_the_names() {
        let home = temp_home("signout-busy");
        let record = home.join("connectors.json");
        sync_steps(&home, &[custom_server("issues")], Some(&record), &mut fake_claude(&home)).unwrap();
        fs::write(state_file(&home), "{ torn").unwrap();
        let mut none = |_: &claude_code::Step| -> Result<(), String> { panic!("nothing runs while the file is unreadable") };
        let removed = sync_steps(&home, &[], Some(&record), &mut none);
        assert!(removed.is_ok(), "a launch waits quietly");
        custom::sign_out(&record, removed.is_ok() && none_held(&home, &record)).unwrap();
        assert_eq!(custom::Record::load(&record).claude_code, BTreeSet::from(["issues".to_string()]), "kept for the next run");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_custom_add_never_goes_through_cmd_exe() {
        let home = temp_home("direct");
        let plain = custom_server("issues");
        let mut quoted = custom_server("wiki");
        quoted.1["note"] = json!("a \"quoted\" & value");
        let mut flags: Vec<(String, bool)> = Vec::new();
        let mut fake = fake_claude(&home);
        let mut run = |s: &claude_code::Step| {
            flags.push((s.args[1].clone(), s.admin_text));
            fake(s)
        };
        sync_steps(&home, &[plain, quoted], None, &mut run).unwrap();
        assert!(flags.iter().all(|(_, direct)| *direct), "both shapes of add_args: {flags:?}");

        // On Windows the direct run is node.exe with Claude Code's script.
        let npm = home.join("npm");
        let script = npm.join("node_modules").join("@anthropic-ai").join("claude-code");
        fs::create_dir_all(&script).unwrap();
        assert_eq!(node_and_script(&npm.join("claude.cmd"), &[]), None, "no script yet");
        fs::write(script.join("cli.js"), "").unwrap();
        assert_eq!(node_and_script(&npm.join("claude.cmd"), &[PathBuf::from("relative")]), None, "no node: not added");
        let nodejs = home.join("nodejs");
        fs::create_dir_all(&nodejs).unwrap();
        fs::write(nodejs.join("node.exe"), "").unwrap();
        let (node, cli) = node_and_script(&npm.join("claude.cmd"), &[nodejs.clone()]).unwrap();
        assert_eq!((node, cli), (nodejs.join("node.exe"), script.join("cli.js")));
        fs::write(npm.join("node.exe"), "").unwrap();
        assert_eq!(node_and_script(&npm.join("claude.cmd"), &[nodejs]).unwrap().0, npm.join("node.exe"), "beside the .cmd first");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn without_node_a_custom_server_is_skipped_with_a_notice() {
        let home = temp_home("no-node");
        let slack = claude_code::connector_servers(&Target { connectors: vec![crate::models::slack("1234567.7654321")], ..Target::default() });
        let wanted = [slack[0].clone(), custom_server("issues")];
        let mut fake = fake_claude(&home);
        let mut run = |s: &claude_code::Step| {
            if s.admin_text {
                return Err(format!("Claude Code's connector {} was not added: Node.js was not found.", s.name));
            }
            fake(s)
        };
        let e = sync_steps(&home, &wanted, None, &mut run).unwrap_err();
        assert!(e.contains("connector issues was not added"), "{e}");
        assert_eq!(held(&home), ["slack"], "the built-in connectors still run");
        let _ = fs::remove_dir_all(&home);
    }

    /// Against the real Claude Code on this machine (`claude` on PATH):
    /// cargo test -- --ignored with_the_real_claude
    #[test]
    #[ignore]
    fn connectors_with_the_real_claude() {
        let home = std::env::temp_dir().join(format!("consus-launcher-real-claude-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let claude = |_: bool| Ok(Command::new("claude"));
        let state = || claude_code::read_object(&state_file(&home)).unwrap();
        let on = Target {
            connectors: vec![crate::models::slack("1234567.7654321")],
            custom: vec![crate::models::CustomServer {
                name: "issues".into(),
                claude_code: Some(json!({ "type": "http", "url": "https://mcp.example.com/mcp", "oauth": { "clientId": "abc123", "callbackPort": 3119 }, "note": "a \"quoted\" value" })),
                claude_desktop: None,
            }],
            ..Target::default()
        };
        let wanted = claude_code::connector_servers(&on);

        sync_claude_connectors(&claude, &home, &wanted).unwrap();
        assert_eq!(state()["mcpServers"]["slack"], wanted[0].1, "Claude Code holds the entry as rendered");
        assert_eq!(state()["mcpServers"]["issues"]["note"], "a \"quoted\" value", "a custom server, as given");
        let owned: Vec<String> = wanted.iter().map(|(n, _)| n.clone()).collect();
        assert!(claude_code::mcp_steps(&home, &wanted, &owned).is_empty(), "the next launch runs nothing");

        // A server of the person's own stays through a change and a removal.
        let mut theirs = Command::new("claude");
        theirs.env("CLAUDE_CONFIG_DIR", claude_code::profile_dir(&home));
        theirs.args(["mcp", "add-json", "theirs", r#"{"type":"http","url":"https://example.com/mcp"}"#, "--scope", "user"]);
        assert!(theirs.output().unwrap().status.success());
        let other = claude_code::connector_servers(&Target { connectors: vec![crate::models::slack("7654321.1234567")], ..Target::default() });
        sync_claude_connectors(&claude, &home, &other).unwrap();
        assert_eq!(state()["mcpServers"]["slack"]["oauth"]["clientId"], "7654321.1234567");
        sync_claude_connectors(&claude, &home, &[]).unwrap();
        let servers = state()["mcpServers"].as_object().cloned().unwrap();
        assert!(!servers.contains_key("slack") && servers.contains_key("theirs"), "{servers:?}");
        let _ = fs::remove_dir_all(&home);
    }
}
