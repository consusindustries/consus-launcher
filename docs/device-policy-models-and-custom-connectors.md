# Device policy: model order and custom connectors

*Addendum to `docs/device-policy.md` and `docs/device-policy-connectors.md`. Drafted
2026-11-16; revised the same day after review (the model setting is order and default
only; custom servers may not carry headers, secrets or variable references; a one-time
notice for a new connector; shared fixtures; a list that places none of a tool's models
leaves that tool as it is; a custom server's values never pass through `cmd.exe`; the
notice's hash defined). The gateway's half is
`gov_open_router/specs/device-policy-models-and-custom-connectors.md` (the two fields and
their rules); the portal's half is `consus-key-portal/specs/launcher-settings-page.md`
(where an admin sets them, and the lock files). This file is the launcher's: two more
values read from the document, one shared function for model order, and custom servers
in the two Claude writers. Planned as 1.6.0.*

## What changes, in three sentences

The org's device policy can now say what order tools list models in and which one they
start on; and it can carry MCP servers the organization wrote itself, as JSON. The
launcher applies the order in every tool it sets up, and writes each custom server into
Claude Desktop and Claude Code the way it writes Slack today, after checking it again.
It keeps a record of the servers it added, so one removed in the portal is removed from
the machine and a person's own servers are left alone, and it tells the person once when
their organization adds or changes one.

## What the system already covers

- **The read, the merge and the cache** (`docs/device-policy.md`): two more fields on the
  same document, read, cached and dropped at sign-out with the rest. The document reader
  ignores fields it does not name, so launchers before 1.6.0 skip `models`; they skip a
  `custom` connector with one log line ("not a kind this launcher knows").
- **Model selection per tool:** `models::claude_models` (opus, sonnet, haiku, newest
  first) feeds Claude Code and Claude Desktop; `models::at_level` feeds Pi, Factory and
  the Codex catalog; `chatgpt::select_model` picks the start model for ChatGPT and Codex.
  All of them read the gateway's `/v1/models` listing for the key, which already holds
  only the models that key may call.
- **Connectors** (`docs/device-policy-connectors.md`): Claude Desktop's entry is rewritten
  whole with `managedMcpServers`; Claude Code is asked to add and remove servers
  (`claude mcp add` / `remove`, user scope) only when its state file differs
  (`claude_code::mcp_steps`). `add_args` already falls back to `claude mcp add-json` for
  an entry that is not the plain OAuth shape.
- **A locked tool:** `tools::org_policy` knows a lock for Claude Code, Claude Desktop and
  Factory; under one the launcher writes nothing for that tool and removes what it wrote
  before. ChatGPT, Codex and Pi are never treated as locked, so the launcher keeps
  writing their settings (Codex's lock, `requirements.toml`, only limits what a setting
  may be).
- **Notices:** the `tool-notice` event, already used when a connector could not be set.

## Decisions

- **The model setting is order and default, nothing else.** It never hides a model.
  Which models a person can use is decided by the organization's model restrictions and
  their key's policy, and the launcher already lists only what the key can call. A model
  the admin did not place, such as one released after they saved, follows the ordered
  ones in the tool's own order.
- **One function applies the order**, so every tool agrees (below).
- **Custom servers are written as given, after the launcher's own check.** The launcher
  fills no slot and adds no blocked tools; what the admin saved is what the tool gets.
  The organization owns those servers (README, below).
- **The launcher repeats every one of the gateway's rules before it writes a file.** The
  gateway checks the keys known today, and it is not the only place a document can come
  from (an organization's own proxy, a hand-edited cache file). The machine that would
  be harmed is the last check. An entry that fails is skipped with one log line naming
  it, never half-written.
- **The launcher owns the names it added, and remembers them.** Today the owned names
  come from the policy document (`slack`). Custom names come from the portal and can go
  away, so the launcher keeps a record (below). As with `slack` today, a server a person
  made themselves under a name the organization then uses is replaced: written down, not
  worked around.
- **A person is told once when a custom server is added or changed.** A portal admin is
  a browser session. The notice means a server pushed from a stolen one does not arrive
  silently.
- **A custom server's values never pass through `cmd.exe`.** On Windows an npm install
  of Claude Code is a `claude.cmd`, which `tools/win.rs` runs through `cmd.exe`. A `"` or
  `&` inside a value an admin typed (a client ID, a display name) can end the quoting
  there and run a command, which is the thing refusing `command` is for. So on that path
  the launcher adds a custom server by starting Claude Code's script with `node` and no
  shell, or it does not add it (below). This is a rule, not something to test and see.
- **Claude Desktop and Claude Code only** for custom servers. ChatGPT, Codex, Pi and
  Factory stay out for the reasons in `docs/device-policy-connectors.md`.
- **Nothing from device management.** There is no `Models` or `Connectors` org setting.
  The device-management path for both is the lock files the portal renders.

## Model order

### The field

```json
"models": { "list": ["claude-sonnet-5-5", "claude-opus-5-5", "gpt-5.6-sol"], "default": "claude-sonnet-5-5" }
```

Ids carry no `consus/` prefix and no level. `settings.rs`: one more field on `Document`,
carried through the merge with its own source (`Portal` or `Default`), and shown on the
Keys tab as a "Model order" row. A `models` value
this launcher cannot read (not an object, `list` missing or empty) is dropped with one log
line and the tools list models as before; it cannot loosen anything, so it does not hide
the tools. Keys in the object other than `list` and `default` are ignored. Ids the key
does not have are skipped silently: that is the normal case for a person whose key policy
is narrower than the org's. `Target` gains `models: Option<ModelChoice>`.

### One function

`models::ordered(candidates, choice) -> Vec<T>`, where `candidates` is what a tool can use
at the level, in that tool's own order, each with its base id:

| Choice | Result |
|---|---|
| None | `candidates`, unchanged (today's behaviour) |
| Some | the candidates that are in `list`, in `list` order; then every other candidate, in the tool's own order |

The result always holds every candidate. So the no-models check is unchanged and a new
model shows up without anyone saving anything.

**A list that places none of a tool's models is no choice, for that tool.** `candidates`
is what this person's key can call in that tool (the listing is already filtered by the
key). When no candidate's id is in `list`, the writer treats the choice as None: the
list, the start model and every slot are what 1.5.1 writes. So an org that orders only
Claude models changes nothing in ChatGPT, Codex, Pi or Factory, and a person whose key
has none of the listed Claude models gets Claude Code as it is today. `ordered` leaves such a list's candidates as they are, and
`start` answers none for it, so the writer keeps its own rule.

`models::start(ordered, choice)`, where `ordered` is already narrowed to the models the
tool can use:

| Choice | Start model |
|---|---|
| None, or none of the tool's models placed | none: the writer uses its own rule, exactly as 1.5.1 does |
| Some, and `default` is in `ordered` | `default` |
| Some, otherwise (no `default`, or the tool cannot use it) | the first of `ordered` |

So an admin who puts Sonnet first and names no default has people start on Sonnet.

### What each tool does with it

| Tool | List | Start model |
|---|---|---|
| Claude Code | `availableModels` is `ordered(claude_models)` | `model` is `start`; with no choice, today's rule (the first opus, else the first sonnet, else the first) |
| Claude Desktop | `inferenceModels` is `ordered(claude_models)`; `isFamilyDefault` marks the first of each family in that order | no key for it in the configuration the launcher writes; see Checks |
| ChatGPT, Codex | the catalog follows `ordered(at_level)` | `start` over the models served over the Responses API; with no choice, or when no placed model is served that way, today's rule (`chatgpt::select_model`'s preference list) |
| Pi | the model rows follow `ordered` | a named `default` Pi can use replaces the person's pick at each launch; otherwise today's rule (a valid pick of the person's stays, else `start`, else the 1.5.1 choice) |
| Factory | the model rows follow `ordered` | `start`; with no choice, today's rule. The medium and light sub-agent models are the first sonnet and the first haiku in `ordered`, each falling back to the start model |

Claude Code's aliases: `ANTHROPIC_DEFAULT_OPUS_MODEL` is the first opus in `ordered`,
`ANTHROPIC_DEFAULT_SONNET_MODEL` the first sonnet, each falling back to the start model
as today. So an admin who puts Opus 5 above Opus 5.5 has chosen what `opus` means. The
background model (`ANTHROPIC_DEFAULT_HAIKU_MODEL`) keeps today's rule exactly: the
preference list, never Sonnet 5. The order does not touch it, because that choice is
about what works for background calls, not what a person picks.
`CLAUDE_CODE_MAX_CONTEXT_TOKENS` follows the start model, as it follows `main` today.

With no choice, or a list that places none of the tool's models, every value above is
what 1.5.1 writes. A test holds each.

## Custom connectors

### The entry

```json
{ "kind": "custom", "name": "issues",
  "claude_code":    { "type": "http", "url": "https://mcp.example.com/mcp", "oauth": { "clientId": "abc123", "callbackPort": 3119 } },
  "claude_desktop": { "transport": "http", "url": "https://mcp.example.com/mcp", "oauth": { "clientId": "abc123", "callbackHost": "localhost" } } }
```

### The check

`custom.rs`, a new module beside `settings.rs`: `settings::checked_connectors` hands each
`custom` entry to `custom::check`. The rules are the gateway's, and its spec is where they are defined; in short, for each blob:

- a JSON object, within the gateway's size and depth limits;
- a `url` that passes `custom::connector_url`, a new check that is the gateway's URL
  rule and nothing else: a string of at most 256 characters, printable with no
  whitespace; `https://`, or `http://` only to `localhost` or `127.0.0.1`; a host of
  letters, digits, `.` and `-`; no username or password; no query string or fragment; a
  port, when given, of digits only and at most 65535 (Python's `urlsplit`, which the
  gateway's validator uses). It only answers yes or no. **It is
  not `normalize_endpoint`**: that function is for the Consus endpoint, strips a trailing
  `/v1` and trailing slashes, and refuses characters the gateway allows in a path. A
  server's `url` is written to the tool exactly as the admin saved it, never rewritten.
  The launcher is stricter in one place: it refuses any character outside printable
  ASCII, where the gateway refuses only what Python calls unprintable;
- none of `command`, `args`, `env`, `headersHelper` at the top;
- no `headers` or `clientSecret` key at any depth;
- no string, key or value, containing `${`. Claude Code replaces `${VAR}` in a server's
  settings with the person's environment variable, so a blob could otherwise send a
  person's credentials to the server it names;
- `type` and `transport`, when present, one of the remote values that client accepts (an
  allowed list, settled by Check 3 below; `stdio` is not on it);
- in the `claude_code` blob, `type` present (decided 2026-11-17, after review): Claude Code
  reads a server without one as a command server and refuses one with only a URL, so
  without this rule the add would fail at every launch. The gateway and the portal refuse
  it at save (fixture case `claude_code_without_type`; header `required_transport_keys`, `connector_transport`).

And for the entry: a name matching `[A-Za-z0-9_-]{1,64}`, not a built-in connector's name
in any case, not already taken by an earlier entry in any case, and at least one of the
two blobs. An entry with neither is skipped with the same log line as any other refusal
(the gateway refuses it too). Claude Desktop shows a custom server under the entry's
name (below), so these rules also keep `managedMcpServers` from holding two servers with
one name; the built-in names include the Desktop names the built-in connectors use
("Slack").

Past `custom_max` custom entries (or `connectors_max` in all), the launcher keeps the
first ones in the document's order and skips the rest, one log line each.

Custom entries take their own path. `settings::connectors()` and `connector_from` fill a
built-in connector's slots from the policy document; a custom entry has no slots and
never goes through them. `checked_connectors` keeps both kinds in one checked list (one
source on the Keys tab), and `parse` splits it into `Target.connectors` and
`Target.custom`. Its "listed twice" rule is for built-in kinds: a second Slack entry is
refused, a second custom entry is not.

**One fixture for three implementations.** `policy/connector-cases.json` lists entries
with the expected outcome: accepted, or refused with the gateway's reason code
(`connector_secret` and so on). Its header carries the limits and the allowed transport
values. It includes the URL edges (256 characters accepted, 257 refused, a bad port, a
bad host character, a path ending in `/v1` accepted and unchanged), so the launcher's
URL check is shown to match the gateway's. This repository owns the file: the launcher's
tests read it, the portal vendors it beside the policy document, and the gateway's tests
carry a copy. A change to the rule changes the fixture here first, and all three suites
follow. The launcher's tests check the code too, as the gateway's do
(`every_shared_connector_case_is_kept_or_refused`).

**Widened in 1.8.0.** A Claude Desktop blob may name one of Claude Desktop's own servers
with `server` and no url, and an entry with no Claude Code part may have single spaces in
its name. See `docs/connectors-registry-launcher.md`. Where this file says "remote servers
only" or gives the name rule, that file is the current one.

The format, the same in all three copies (the gateway wrote it first, 2026-11-16):

```json
{
  "owner": "consus-launcher/policy/connector-cases.json",
  "transports": { "claude_code": ["http"], "claude_desktop": ["http"] },
  "transport_keys": ["type", "transport"],
  "command_keys": ["command", "args", "env", "headersHelper"],
  "secret_keys": ["headers", "clientSecret"],
  "variable_marker": "${",
  "blob_max_bytes": 4096, "blob_max_depth": 8,
  "custom_max": 16, "connectors_max": 24, "url_max": 256,
  "cases": [
    { "id": "claude_code_blob_only", "outcome": "accepted",
      "connectors": [ { "kind": "custom", "name": "issues", "claude_code": { "type": "http", "url": "https://mcp.example.com/mcp", "oauth": { "clientId": "abc123", "callbackPort": 3119 } } } ] },
    { "id": "headers_top_level", "outcome": "refused", "reason": "connector_secret",
      "connectors": [ { "kind": "custom", "name": "issues", "claude_code": { "type": "http", "url": "https://mcp.example.com/mcp", "headers": { "X-Key": "k" } } } ] }
  ]
}
```

- `connectors` is a whole `connectors` list, not one entry, so a case can hold two
  entries (a repeated name, a name that differs only in case, the 17th custom entry).
- `outcome` is `"accepted"` or `"refused"`; a refused case gives the gateway's `reason`
  for the first rule broken. The gateway refuses the whole list with that code. The
  launcher keeps what it can, so its test is: an accepted list is kept whole, and a
  refused one loses at least one entry.
- Every key before `cases` is the header, and each implementation asserts its own
  constants equal it. `http` is the only transport in v1 (proven by hand in both clients
  2026-11-11); another value is added only after a hand check, with the constant.
- Secret keys match without regard to case, `_` or `-` (`client_secret` is refused).
  `type` and `transport` are both checked in both blobs, against that blob's list.
- A long value is written out in full, not generated, so the three suites read the same
  bytes.
- `blob_max_bytes` is the UTF-8 length of the blob as compact JSON, non-ASCII characters
  not escaped. `blob_max_depth` counts the blob itself as depth 1, and each object or
  array inside it one more. A `null` blob is the same as an absent one.
- A `name` inside a `claude_desktop` blob is allowed and checked by nobody: the launcher
  and the portal's lock files replace it with the entry's (decided 2026-11-17).

`Target` gains `custom: Vec<CustomServer>`, each a name and up to two blobs.
`policy/launcher-policy.json` gains nothing for a custom server: nothing about one is
Consus's to decide.

### Claude Desktop (`config.rs`)

Each entry with a `claude_desktop` blob is appended to `managedMcpServers`, after the
built-in connectors. Claude Desktop shows the entry's name; a `name` in the blob is
replaced. The launcher's entry is
already rewritten whole at each launch, so a server removed in the portal is gone at the
next one. Nothing under the Desktop lock, as today.

### Claude Code (`claude_code.rs`)

Each entry with a `claude_code` blob is a wanted server under the entry's name, beside
the built-in ones, through the same `mcp_steps` comparison: nothing is run when Claude
Code already holds it. `add_args` keeps its two shapes: a blob in the plain OAuth shape
goes as flags, anything else as `add-json`.

**Windows, npm install.** Only the `claude.cmd` path is affected; the native `claude.exe`
has no shell in between and runs as on macOS. On the `claude.cmd` path, the add for a
custom server (either shape, since a client ID passed as a flag is just as much the
admin's text as a blob) is run as `node.exe <script> mcp add…` with no shell:

- the script is `node_modules\@anthropic-ai\claude-code\cli.js` beside the `.cmd`;
- `node.exe` is looked for beside the `.cmd` first, then in each `PATH` folder, and run
  by that full path, never as a bare `node.exe` that Windows would also look for in the
  launcher's own folder;
- it runs with what `claude_command` gives the run today: no console window, and the
  same environment;
- if either is missing, the server is not added and the tile notice says so. Nothing is
  half-written, and the built-in connectors are unaffected.

Nothing else changes on that path: `remove` carries only the name, which the check has
already limited to letters, digits, `_` and `-`, and Slack's add keeps going as it does
today.

No deny rules are written for a custom server. Claude Code has no per-server tool policy,
and a list of tools to block is not part of the entry in v1 (Out of scope).

### The record

`connectors.json` in the launcher's own folder:

```json
{ "claude_code": { "issues": "9d04be71c2aa", "wiki": "" }, "seen": { "issues": "3f9a1c0b7d2e", "wiki": "a41b…" } }
```

No value from a blob is in it.

- **`claude_code`:** the names the launcher asked Claude Code to add. The names the
  launcher owns in Claude Code become: the policy document's keys, these names, and the
  names wanted now. A name is written here before its `add` runs, and taken out after the
  state file shows it gone. A crash between the two leaves a name Claude Code does not
  hold, which costs one `remove` that answers "no server named". A recorded name that is
  no longer wanted is removed from Claude Code. A server in neither list is never named
  in any run.

  Each name carries the hash (as for `seen`) of the blob Claude Code was last seen to
  hold from it, empty until then. Claude Code keeps only the fields it knows (2.1.326
  drops the rest, found by the real-tool test 2026-11-18), so what it holds never equals
  a blob with more, and comparing the two would remove and re-add such a server at every
  launch. A custom server therefore counts as up to date when Claude Code holds its name
  and the recorded hash is this blob's. The hash is recorded only when every field
  Claude Code kept is the blob's own, so an older copy that a failed remove left behind
  is not taken for the new one. Built-in connectors keep the field-by-field comparison.
- **`seen`:** per custom server, the first 12 hex characters of the SHA-256 of
  `[name, claude_code, claude_desktop]` written as compact JSON, with object keys sorted
  at every level and a missing blob as `null`. The launcher sorts the keys itself and
  does not lean on the JSON library's map order. So a reordered or respaced document is
  not a change, and adding a Desktop blob to an existing server is. Used only for the
  notice below, and compared only with this launcher's own earlier value.
- Sign out removes every recorded name from Claude Code, then the file. If Claude Code
  could not be asked (not installed, its state file busy), the names stay and only
  `seen` goes, so the next run still removes them. `seen` goes with
  it, so after the next sign-in each custom server is announced once more. Under the
  Claude Code lock the launcher removes its recorded names and adds none.

### The notice

When the document carries a custom server whose hash is not in `seen` (new, or changed),
the launcher shows one notice the next time its window is in front, on the tile of each
tool the server is written to (a person who only uses Claude Desktop sees it there): "Your organization
added the connector issues (mcp.example.com)." or "…changed the connector…". It names
the server and the host of its URL, nothing else (from 1.8.0, the Claude Desktop server
it names where it has no URL: "Microsoft 365 (microsoft365)"). The hash is recorded when the notice is
shown, so it appears once per person per change. A server that fails the check gets no
notice and no entry; the log line says why.

It is for detection: the server is written at the next launch whether or not the notice
has been shown yet; the Keys tab lists it at all times.

The notice is about what the launcher writes, so it follows the tools without a lock:

- A server is noticed when the launcher will write it into at least one tool: it has a
  blob for Claude Code or Claude Desktop, and that tool is set up and not under a lock.
- On a machine where both are under a lock, or for a server whose only blob is for a
  locked tool, there is no notice and nothing is put in `seen`. The server reaches that
  tool only when IT pushes the lock files, which a portal session alone cannot do.
- If a lock is later removed, a server not yet noticed is written and noticed then, as
  new. One already noticed for the other tool is not noticed again.
- After sign-out and sign-in, each server is noticed once more (`seen` went with the
  record).

A tile's notice stays through the launcher's refreshes and is cleared when that tool is
next opened, as the existing tile notices are.

The Keys tab's connector list names custom servers by their `name` and host (from 1.8.0, the
Claude Desktop server one names where it has no URL), after the built-in ones. A failed add is the existing tile notice, naming the server.

## Checks before the build

1. **Windows, npm install: where the script and `node` are.** The rule is decided
   (Decisions; "Windows, npm install" above). What the Windows VM settles is only the
   lookup: that `cli.js` is where this file says for the current npm release, that
   `node.exe` is found for the common installers (the Node installer, nvm-windows, a
   per-user npm prefix), and that an add run that way lands in the same state file.
2. **Claude Desktop's start model.** Find out whether the picker starts on the first
   entry of `inferenceModels` or on the family default. The portal's help text for the
   default follows the answer. Answered 2026-11-18 on the Windows VM (Claude Desktop
   2.9939, under the portal's lock): it starts on the first entry; the default is not
   moved up for it (decided the same day). The portal's help text says "Claude Desktop
   starts on the first model in the order".
3. **The remote transports each client accepts.** For Claude Code's `type` and Claude
   Desktop's `transport` in a managed entry: which values other than `http` work (`sse`,
   any other), by trying each. The allowed list in the gateway, here and in the portal
   is exactly the values that pass, and the fixture records them. Done for Claude Code
   2.1.326 on macOS (2026-11-18, `a_server_without_type_is_refused_by_the_real_claude`):
   a server with only a URL is refused, one with `type: http` is taken.
4. **`availableModels` order.** Confirm Claude Code's `/model` picker follows the order
   of `availableModels` (the current release). If it sorts on its own, the portal says
   the order applies to the other tools only.
5. **Variable references.** Confirm on a real machine that Claude Code and Claude Desktop
   leave a server alone when nothing in it contains `${`, and note any other
   substitution syntax either supports; anything found joins the refusal. Claude Code
   stores `$HOME`, `%USERPROFILE%` and `${HOME}` as written (2026-11-18,
   `variables_are_stored_as_written_by_the_real_claude`); it replaces `${VAR}` and
   `${VAR:-default}` only when it connects, per its documentation, and both start with
   the refused `${`.

`scripts/real-tools-test.sh` runs these against the Claude Code installed on this Mac
and on the Windows VM, with the rest of the tests, whenever needed (its header says
when).

## Documents customers audit

- **README, "Tools":** model order (what it does; that it never hides or grants a model;
  to remove a model, restrict it in the portal) and custom connectors (written as the
  organization saved them; the organization is responsible for the servers it adds; the
  launcher adds no blocked tools for them; remote servers only, with no headers, secrets,
  variable references or query string in the URL, so a server that carries a tenant in
  the query cannot be added; the one-time notice, and that signing out and back in shows
  it again for each server; that a server a person made themselves under a name the
  organization then uses is replaced, and is gone once the organization removes that
  name; that on Windows with an npm install of Claude Code a custom server needs `node`
  to be found, and is skipped with a notice when it is not).
- **SECURITY.md:** `connectors.json` under "Files it writes" and that sign out removes
  it; the check the launcher runs on a custom server before writing it; one sentence that
  a custom server's traffic goes from the machine to that server and never through
  Consus.
- **`docs/consus-launcher-handoff.md`:** the two fields in the section on the document.
- **CHANGELOG**, 1.6.0: both features in a person's words, and the line IT needs: "On a
  machine with a Claude lock file, the model order and connectors for Claude come from
  the lock; push the lock files again after changing either in the portal."

This repository is public: none of these documents, or this file, names a customer.

## Tests

`models.rs`:
- `no_choice_changes_nothing`; `the_list_orders_and_the_rest_follow`;
  `a_new_model_follows_the_ordered_ones`; `ids_the_key_lacks_are_skipped`;
  `the_default_starts_only_when_the_tool_has_it`;
  `a_choice_without_a_default_starts_on_the_first_listed`;
  `no_choice_gives_no_start_model`;
  `a_list_that_places_none_of_a_tools_models_keeps_its_start`.
- Every case in `policy/model-choice-cases.json`: candidates, a choice, the expected
  order and start, **and Claude Code's slots** (`model`, the opus and sonnet aliases, the
  background model, the context tokens). One case is a list that places none of the
  candidates, with every slot expected to equal the no-choice case. The portal runs the
  same file, so the lock files and the launcher cannot drift apart on those rules.

`claude_code.rs`:
- `the_order_decides_available_models_and_the_aliases`: sonnet placed first, Opus 5
  above Opus 5.5: `model` is the default, the opus alias is Opus 5.
- `the_background_model_ignores_the_order`.
- `with_no_choice_the_settings_are_what_1_5_1_wrote`.
- `a_custom_server_is_added_and_recorded`, `removed_in_the_portal_it_is_removed_here`,
  `a_persons_own_server_is_never_named`, `sign_out_removes_recorded_names_and_the_record`,
  `under_the_lock_recorded_names_go_and_none_are_added`.
- `a_custom_add_never_goes_through_cmd_exe`: on the `claude.cmd` path, for both shapes of
  `add_args`, the command built is `node.exe` with the script, or no command at all.
- `without_node_a_custom_server_is_skipped_with_a_notice`: the built-in connectors are
  still added.

`config.rs`: `custom_servers_follow_the_built_in_ones`; `a_blob_without_a_name_gets_the_entrys`;
`the_order_decides_inference_models_and_family_defaults`.

`settings.rs`:
- Every case in `policy/connector-cases.json`.
- `a_models_value_it_cannot_read_is_dropped_not_fatal`; `unknown_keys_in_models_are_ignored`.
- `a_new_or_changed_server_is_noticed_once`; `a_refused_server_is_never_noticed`;
  `reordered_keys_are_not_a_change`; `an_added_blob_is_a_change`;
  `an_entry_with_neither_blob_is_skipped`; `custom_entries_skip_the_slot_checks`;
  `a_name_in_the_desktop_blob_is_allowed_and_not_checked`; `a_name_in_the_blob_is_replaced_by_the_entrys`;
  `a_url_ending_in_v1_is_written_as_given`; `nothing_is_noticed_under_both_locks`;
  `a_server_for_a_locked_tool_only_is_not_noticed`; `the_fixture_header_equals_the_constants`.

`chatgpt.rs`, `pi.rs`, `factory.rs`: one test each that the order decides the rows and
the default starts when the tool can use it, and one each that a Claude-only list leaves
the rows and the start model as 1.5.1 wrote them.

## Release

1. The five checks above, on real machines.
2. One PR, reviewed, CI green.
3. Real-machine check on macOS and Windows against the deployed gateway: set an order in
   the portal, refocus, open each tool and see the order and the start model; add a
   custom server, see the notice once, open Claude Desktop and Claude Code, connect;
   change its URL in the portal, see the notice again; remove it, reopen, it is gone and
   a server made by hand is still there; sign out, `connectors.json` is gone. On Windows,
   do the custom-server steps once with an npm install of Claude Code (the `node` path)
   and once with the native `claude.exe`.
4. Tag and publish 1.6.0, then the portal's `LAUNCHER_VERSION`, the vendored policy
   document and fixtures, and `latest.json`.

## Out of scope

- Hiding a model. The organization's model restrictions do that, for the tools and the
  API alike.
- Command (stdio) servers, and servers that need a header, a static token or a client
  secret. The gateway refuses them and so does the launcher.
- Blocked-tool lists for custom servers in Claude Code. The server's own grant is the
  control. If wanted later, it is one optional list on the entry, rendered as deny rules
  the way Slack's are.
- Custom servers in ChatGPT, Codex, Pi or Factory.
- A different model order per tool or per group.
- Writing anything for a tool under its lock. Whether a lock should carry only the
  compliance settings and leave the model order to the launcher is a separate question
  (the portal spec, "Later").
