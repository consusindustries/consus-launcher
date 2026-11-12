# Device policy: connectors

*Addendum to `docs/device-policy.md`. Drafted 2026-11-11, after Slack's hosted MCP server
was connected from Claude Desktop (gateway mode) and Claude Code with one Slack app the
organization registered, no secret. The gateway's half is
`gov_open_router/specs/device-policy-connectors.md` (the field); the portal's half is
`consus-key-portal/specs/device-policy-connectors.md` (the row and the lock files). This
file is the launcher's: one more slot read from the document, two writers, and the policy
document section both the launcher and the portal render from. Ships as 1.4.0.*

## What changes, in three sentences

The org's device policy can now name a connector: for Slack, the client ID of the Slack app
the organization registered. The launcher writes Slack into Claude Desktop and Claude Code
from that one value, with everything Consus decides (server, ports, blocked tools) coming
from the policy document, so each person only clicks Connect once. The same document
section is what the portal renders into the lock files, so a locked machine and an
unlocked one get the same Slack.

## What the system already covers

- **The document read and merge** (`docs/device-policy.md`): one more field on the same
  document, read with the rest, cached with the rest, dropped at sign-out with the rest.
- **The policy document** (`policy/launcher-policy.json`, `src-tauri/src/policy.rs`): parsed
  strictly, embedded, vendored by the portal. The connector section goes there, and the
  portal's lock renderers read it.
- **Owned keys** in each writer: Claude Code's `write_settings` removes and rewrites only
  the keys the launcher owns (`OWNED_TOP`, `OWNED_ENV`, the deny rules, the sandbox paths)
  and leaves the person's other settings; Claude Desktop's `write_claude_desktop` rewrites
  the launcher's own config-library entry. Desktop's connector follows that pattern; Claude
  Code's blocked tools follow the deny-rule pattern. Claude Code's server entry itself is
  not a settings key: it lives in Claude Code's own state file, which the launcher does
  not write; it asks Claude Code's CLI to add it (below).
- **A locked tool**: `tools::org_policy` already knows when a tool's device-management lock
  is present; for Claude Desktop the launcher then writes nothing (`Tool::Desktop if
  policy`).
- **Verified by hand, 2026-11-11, on macOS:** Claude Desktop with a managed entry
  (`transport http`, `oauth.clientId`, `callbackHost localhost`): "Connected successfully".
  Claude Code in the launcher's profile with `claude mcp add-json slack '{"type":"http",
  "url":"https://mcp.slack.com/mcp","oauth":{"clientId":"…","callbackPort":3118}}'
  --scope user`, then `/mcp` → Authenticate: connected. Both with the same Slack app,
  PKCE on, redirect URLs `http://localhost:53280/callback` and
  `http://localhost:3118/callback`. Desktop's `toolPolicy` shape is the app's own: it came
  out of Claude Desktop's Export after blocking tools in its UI.

## Decisions

- **Slack only in 1.4.0; Claude Desktop and Claude Code.** ChatGPT and Codex are out: Codex
  builds its OAuth redirect on a random loopback port and path, which a Slack app cannot
  register (tested; Slack answers "redirect_uri did not match"). Pi is out: MCP in Pi is an
  extension a person installs, and the launcher installs nothing.
- **The document section is the single source.** The launcher renders its two entries
  from it; the portal renders the two lock entries from the vendored copy. No URL, port,
  or tool name is typed anywhere else.
- **The launcher owns each tool's connector list.** Claude Desktop: the launcher's
  config-library entry, which it already rewrites whole; a connector a person added by
  hand in that entry does not survive the next launch, which is deliberate and now written
  down (test a new connector with Claude opened directly, not from the launcher). Claude
  Code: the launcher asks Claude Code to add the server, with `claude mcp remove` then
  `claude mcp add-json --scope user` in the launcher's profile, so Claude Code writes its
  own state file in its own format. The launcher owns the server names the document
  knows (`slack`) and nothing else; the person's other servers stay. A server the person
  named `slack` themselves is one the launcher replaces, or removes when the portal has no
  Slack: written down, not worked around.
- **A locked tool gets its connector from the lock, not from the launcher.** Under a
  lock the launcher removes and never adds: on Claude Code it still removes the server
  names it owns and the blocked-tool deny rules, so a machine that ran unlocked before IT
  pushed the lock does not show Slack twice; it writes nothing new. The lock carries the
  whole entry (the portal's rule: a lock is a snapshot): for Claude Code that is
  `managedMcpServers` in `managed-settings.json` (Claude Code 2.1.259 and later; not
  `managed-mcp.json`, the exclusive-control file, which Consus does not use), plus the
  same deny rules. On Claude Desktop a profile makes the launcher's entry invisible, and
  the launcher already writes nothing for a locked Desktop.
- **Read-only is the Slack app's scopes.** The document blocks Slack's write tools on the
  client as a second layer, in each client's own form: `toolPolicy` in Desktop's entry,
  `permissions.deny` rules `mcp__slack__<tool>` in Claude Code's settings. The scopes are
  the control, and they are the organization's. The blocked list is best effort: Slack can
  add a write tool before the list knows it, so the list is reviewed each release.
- **An unknown kind or a bad client ID is skipped, not fatal.** The gateway already
  validated the field; a kind this launcher's document does not know is a newer portal,
  and the right outcome is "no connector" with a log line, not "no tools". The rest of the
  document applies.
- **Nothing from device management.** There is no `Connectors` org setting for the
  launcher. The device-management path for connectors is the lock files.

## The document section

`policy/launcher-policy.json`, schema stays 1; the section is optional and both parsers
(`policy.rs`, the portal's `policy.ts`) treat its absence as "no connectors".

```json
"slots": { "...": "...", "slack_client_id": { "kind": "slack-client-id", "note": "The Slack app's client ID (<team>.<app>), from the org's device policy." } },
"connectors": {
  "slack": {
    "name": "Slack",
    "server": "https://mcp.slack.com/mcp",
    "about": "…",
    "slot_keys": { "client_id": "slack_client_id" },
    "blocked_tools": [
      "slack_send_message", "slack_schedule_message", "slack_send_message_draft",
      "slack_add_reaction", "slack_create_conversation", "slack_create_list",
      "slack_update_list", "slack_add_list_record", "slack_update_list_record",
      "slack_create_canvas", "slack_update_canvas",
      "slack_get_file_upload_url", "slack_complete_file_upload"
    ],
    "tools": {
      "claude-desktop": {
        "entry": {
          "name": "Slack", "transport": "http", "url": "https://mcp.slack.com/mcp",
          "oauth": { "clientId": "$slack_client_id", "callbackHost": "localhost", "callbackPort": 53280 },
          "toolPolicy": { "$blocked_tools": "blocked" }
        }
      },
      "claude-code": {
        "key": "slack",
        "entry": {
          "type": "http", "url": "https://mcp.slack.com/mcp",
          "oauth": { "clientId": "$slack_client_id", "callbackPort": 3118 }
        },
        "allow": { "serverUrl": "https://mcp.slack.com/*" }
      }
    },
    "redirect_urls": ["http://localhost:53280/callback", "http://localhost:3118/callback"]
  }
}
```

- `slot_keys` maps the device-policy entry's fields to slots (Slack: `client_id` fills
  `slack_client_id`), the same map the portal's renderer uses, so a later kind with two
  fields (`m365-local`: `tenant_id`, `client_id`) needs no new code. The slot's `kind`
  (`slack-client-id`) names the check its value must pass; a slot kind this launcher does
  not check is never valid, so its connector is skipped.
- `$slack_client_id` is a slot, filled like `endpoint` and `helper`. `$blocked_tools`
  expands to one `"<tool>": "blocked"` pair per name, so the list is written once. Claude
  Code's form is derived, not written: one deny rule `mcp__<key>__<tool>` per name, which
  the launcher adds to its owned deny rules and the portal puts in the lock.
- `callbackPort` is explicit for both clients, so the `redirect_urls` the portal's
  manifest shows never rest on a client's default.
- `allow` is what the portal adds to Claude Code's `allowedMcpServers` in the lock (by
  URL, never by name). Managed servers load without it and the launcher adds nothing under
  a lock, so it is belt-and-braces, kept because it is cheap. The launcher does not write
  it: there is no allowlist in user settings.
- `key` is the Claude Code object key, the same one the portal uses in the lock.
- `redirect_urls` is read by nobody at run time; it is the record the portal's manifest
  card and the README copy from, so the three never disagree.
- `policy.rs` gains `Connector` / `ConnectorTool` structs under `deny_unknown_fields`;
  `connector_from(entry)`, which checks a device-policy entry; `connector_entry(connector,
  tool) -> Option<Value>`, which fills the slots; `connector_keys(tool)`, the server names
  the launcher owns; and `connector_deny(kind) -> Vec<String>` for Claude Code's rules. A test asserts every
  `blocked_tools` name appears in Desktop's rendered `toolPolicy` and in Claude Code's deny
  rules, and that `redirect_urls` matches the two `callbackPort`s.

## The slot

`settings.rs`: the document's `connectors` list (each `{ "kind", "client_id", … }`) is one
more field on `Document`, carried through `merge` untouched (device management never
sets it), with its own source `Portal` or `Default`. Validation per entry, in
`check_document`: `kind` must exist in the policy document's `connectors` section and
`client_id` must match `^\d{6,20}\.\d{6,20}$`; otherwise the entry is skipped with one log
line naming the kind (never the value). A second entry of a kind already taken is skipped
the same way (the gateway refuses those). Extra fields are ignored: a newer portal may add
optional ones. `Target` gains `connectors: Vec<Connector>` (kind and the slot fields), so
every writer gets them the way it gets `web_access`.

## The writers

**Claude Desktop** (`config.rs`): `write_claude_desktop` adds `managedMcpServers` to the
launcher's entry, the rendered `entry` per connector, when there is at least one; the key
is absent otherwise. The entry is already rewritten whole, so a connector the portal
dropped is gone at the next launch. Skipped when the Desktop lock is present (as today).

**Claude Code** (`claude_code.rs`), two parts:

- *The server:* the launcher asks Claude Code to add it. For a connector key that holds
  something other than what the policy names, `claude mcp remove <key> --scope user`;
  then for each one the policy names, `claude mcp add --transport http <key> <url>
  --client-id <id> --callback-port <port> --scope user`. Claude Code stores that exactly
  as `add-json` would (checked, 2.1.322), and the command line carries no quote: on
  Windows an npm `claude.cmd` runs through `cmd.exe`, which mangles quoted JSON when the
  path has a space. An entry outside that shape falls back to `add-json`. Each run has
  `CLAUDE_CONFIG_DIR` set to the profile (`~/.claude-consus-gateway`), `~/Consus` as its
  working folder (Claude Code reads the folder it runs in as a project), the
  launcher's own `CLAUDE*` / `ANTHROPIC*` variables scrubbed, the policy's no-traffic env
  switches set, and a 20-second limit. It runs before the terminal opens, with the
  `claude` binary the launcher already finds. Before running anything the launcher reads
  Claude Code's state file (read-only) and runs nothing when the servers under its names
  already match (`mcp_steps`: the held entry carries every rendered field with the same
  value; a field Claude Code adds of its own is no difference), so an ordinary launch
  costs no run; afterwards it reads the file again, and a mismatch is the notice. A state
  file that cannot be read (a running session midway through rewriting it) means nothing
  is done that launch. Claude Code writes
  its own state file in its own format, and a session already running in that profile is
  something `claude mcp add` is built to coexist with. A failed add is a tile notice
  ("Slack could not be added to Claude Code"), not a failed launch.
- *The blocked tools:* `mcp__<key>__<tool>` rules join the launcher's owned deny rules in
  `settings.json`, through `set_deny`, so `remove_settings` already removes them.

Sign out runs the same removal, off the UI thread. Under the Claude Code lock the removal
runs, the connector deny rules are taken out of `settings.json` (`remove_connector_rules`,
nothing else in the file changes), and nothing is
added. The Claude Code `/mcp` screen then shows Slack with "Needs authentication" until
the person authenticates once.

Neither writer touches OAuth tokens: both apps keep those in their own stores. Whether
Claude Code keeps a server's token across `remove` and `add-json` is Claude Code's
business; the launcher only runs them when the entry changed (a new client ID, Slack
turned on or off), when signing in again is expected anyway.

**No traffic to Anthropic from `claude mcp`.** Run bare, `claude mcp add-json` and
`remove` call `api.anthropic.com` (two requests through a logging proxy, Claude Code
2.1.322). With the policy document's Claude Code env switches set on the process
(`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` and the rest of `tools.claude-code.env`) they
make none. The runner sets them on every run, so the claim does not rest on the profile's
`settings.json` having been written first (it is not, under a lock).

## After a read

Unchanged from `docs/device-policy.md`: the UI reloads, `detect_tools` re-runs, and a
changed connector list applies at the next launch of each tool. The Keys tab's "Settings"
block gains one line, `Connectors: Slack · From the Consus portal`, or `Connectors: None`.

## Documents customers audit

- **README:** under "Tools", one paragraph: the launcher writes the connectors the
  organization chose in the portal into Claude Desktop and Claude Code; each connector
  runs where its vendor runs it (Slack's at `mcp.slack.com`) and talks to the laptop,
  never through Consus; what Claude reads from it goes to the model through the gateway
  like any prompt; the Slack app's read scopes are the control and the launcher blocks the
  write tools as well, as a best effort reviewed each release. A sentence that the
  launcher owns the connector list in Claude Desktop's launcher entry, and in Claude Code
  asks Claude Code to add or remove the server names it knows (`slack`), which replaces a
  server the person named `slack` themselves, and leaves other servers alone. "What it connects to" is unchanged: the launcher itself makes no new request;
  the tools do.
- **SECURITY.md:** "Files it writes" gains one line: Claude Code's own user config in the
  launcher's profile, via Claude Code's CLI (`claude mcp add` / `remove`), for the server
  names the document knows; sign out removes them. The deny rules are in `settings.json`,
  already listed.
- **Troubleshooting (new section in the README):** Connect fails with a redirect error:
  the Slack app's redirect URLs must be exactly `http://localhost:53280/callback` (Claude
  Desktop) and `http://localhost:3118/callback` (Claude Code), saved (the Slack page has a
  separate Save button), `localhost` not `127.0.0.1`. Connect fails at once: something else
  on the laptop is listening on 53280 or 3118. Slack is missing on a locked machine: the
  lock carries it; IT pushes the lock again after setting the client ID.
- **`docs/consus-launcher-handoff.md`:** this file linked from section 2; section 7's
  table notes the connector keys per tool.
- **CHANGELOG:** the entry.

The Codex model-catalog fix first drafted here shipped on its own as 1.3.1 (PR 69):
ChatGPT's engine (Codex 0.160) dropped `supports_parallel_tool_calls`, Codex 0.147 needs
it, and the launcher now fills it in as `true`, the value 0.147's own entries carry.

## Tests

`policy.rs`: the section parses; `connector_entry("slack", "claude-desktop", id)` carries
the id, `callbackHost`, `callbackPort` 53280 and all thirteen blocked tools;
`("slack", "claude-code", id)` carries port 3118 and the same tools; an unknown kind is
`None`; `redirect_urls` matches the ports.

`settings.rs`: a document with `[{kind:"slack",client_id:"1234567.7654321"}]` yields one
connector; `kind:"github"` is skipped with the rest applied; a bad `client_id` is skipped;
`[]` and absent yield none; the Keys-tab line reads as specified.

`config.rs`: the entry carries `managedMcpServers` with one Slack when set and no such key
when not; a second write with no connectors removes it.

`claude_code.rs`: the CLI calls for one Slack are an `add` with plain flags and `--scope
user`; a changed client ID is a remove then the add; with no connectors, only the remove;
an extra field Claude Code added, or an unreadable state file, asks for nothing; an entry
off the plain shape goes as `add-json`. Under the lock the connector deny rules go and
nothing else in `settings.json` changes. The deny rules carry all thirteen `mcp__slack__` names when Slack is set,
none when it is not, and a person's own deny rules survive both.
`the_owned_keys_are_exactly_what_the_policy_document_describes` extends to the connector
deny rules.

`tools.rs`: `connectors_with_the_real_claude` (ignored by default; `cargo test -- --ignored
with_the_real_claude`) runs the real runner against the `claude` on PATH in a temporary
profile: Slack added as rendered, a second run asks for nothing, a changed client ID
replaces it, an empty list removes it, and a server of the person's own survives all of
it. Passed on Claude Code 2.1.322, 2026-11-11. The lock path is the empty list (the
`mcp_steps` tests and this one); the Code-locked real-machine check below covers it end to
end.

## Release

1. One PR, reviewed, CI green: the section, the slot, the two writers, the documents
   above.
2. Real-machine check against the deployed portal, macOS and Windows:
   - No lock: set the client ID in the portal, refocus the launcher, open Claude Desktop
     (Slack in Connectors, Connect once, ask about Slack) and Claude Code (`/mcp` shows
     slack, Authenticate once, ask about Slack); clear the ID in the portal, refocus,
     relaunch both, Slack is gone from both.
   - Windows: Claude Code shows `slack` in `/mcp` after a launch from the launcher (npm's
     `claude.cmd` if that is how it is installed; the add uses plain flags, no quotes).
   - In both clients, ask Claude to send a Slack message: it is refused (Desktop by
     `toolPolicy`, Claude Code by the deny rule), and Claude can still read.
   - Claude Desktop lock installed: Slack comes from the profile; the launcher writes no
     entry.
   - Claude Code lock installed (with the portal's allowlist entry) on a machine that ran
     unlocked first: Slack comes from the lock, the launcher removes its own and adds
     none, `/mcp` shows one Slack.
   - **Done 2026-11-11, macOS, against the deployed gateway and portal (PR 70's build):** the
     Keys tab showed "Connectors: Slack · From the Consus portal"; Claude Desktop's applied
     entry held one Slack (port 53280, 13 blocked tools), Claude read the latest Slack
     message, and a send was refused ("my Slack tools here only let me read and search");
     Claude Code 2.1.322 read Slack through the launcher's `slack` server with the 13
     `mcp__slack__` deny rules in its settings. Removal, the lock path and Windows rest on the
     tests, the real-`claude` test and CI.
3. Tag and publish 1.4.0 (minor), then the portal's vendor-bump PR, which turns on the lock
   files' half and flips the Launcher page's note.

## Out of scope

- Other kinds (`m365-local`, `github`): each a new entry in the document's `connectors`
  section and a line in the gateway's accepted set; no new writer code if the shape holds.
- Connectors for Codex, ChatGPT, Pi (above).
- A device-management setting for connectors in the launcher: the lock files are that path.
- Removing a person's OAuth tokens: the apps own those.
