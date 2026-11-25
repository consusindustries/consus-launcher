# Connectors: the launcher's part of the first release

*Context for whoever plans the launcher build. Drafted 2026-11-24 from a read of this
repository at v1.7.0 (`main`, nothing uncommitted). The guiding document is
`consus-key-portal/specs/connectors-registry.md`; it sets the direction and defines the
first release, and it wins where this file and it differ. The gateway and the portal each
have their own file of this kind. This one is the launcher's: what has to change here,
why, where it lives today, and what is not yet known. It is not a build plan and holds no
code. Builds on `docs/device-policy-connectors.md` (Slack) and
`docs/device-policy-models-and-custom-connectors.md` (custom connectors, 1.6.0). Planned
as 1.8.0.*

*This repository is public: this file names no customer.*

*Built since, for 1.8.0. Every "Today" below is the code at v1.7.0, before this change,
and the table in (c) shows what 1.7.0 code would have shown. After it: the fixture has
91 cases, `CustomServer::host` is `CustomServer::shown`, and a spaced name refused for
its Claude Code part is named in the log line.*

## Decided after this was written (2026-11-24)

These are now in the guiding document, and the sections below are brought in line with
them.

- **Two name rules, not one.** Confirmed. The strict rule stays wherever a
  name can reach Claude Code or a command line, `Record::load` included. The wider rule
  (single spaces) is a second rule for Desktop-only entries, and the entry must know its
  parts before its name is checked. The guiding document's wording is corrected to match.
- **Silent skip on old launchers.** Accepted as it is in the launcher. The portal will
  tell the admin that laptops below 1.8.0 do not get such a connector until they update.
  No launcher change for this.
- **An imported entry is kept as exported**, values equal to Claude's defaults included.
  The launcher writes what it is given, so nothing changes here.
- **The gateway re-checks names when it reads a stored document**, and will widen that
  check too. Until it deploys, a spaced name never reaches a launcher.
- **Shapes accepted today that the new rule refuses.** The gateway side
  scans stored documents before its deploy.
- **The 4,096-byte limit.** Kept. The portal shows a clear message when an entry is over.

- **The fixture details are settled** (guiding document, "The exact rules, for the
  fixture"). In short: an entry is a built-in when its Claude Desktop part has the key
  `server`; the value is 1 to 64 letters, numbers, `-` or `_`; one new reason code,
  `connector_server`, covers a bad value, a built-in with `url` or a transport key, and
  `server` in a Claude Code part; the check sits after `command` and before secrets, and
  the transport and url checks run only when there is no `server`; a Desktop-only name is
  words joined by one ordinary space, 64 characters at most, and a spaced name on an entry
  with a Claude Code part is refused with `connector_name`; the name is still checked
  first; the fixture's header gains `server_key`, and the patterns are pinned by cases.
- **Every accepted built-in case in the fixture uses `server: "microsoft365"`.** The
  portal refuses a value it has no form for in the same function that runs the fixture.
  That any value of the right shape is accepted is proved in the launcher's own tests,
  not in the shared file.

- **What a mixed entry shows.** The notice and the Keys tab show the url's
  host when either part has a url. The `server` value is shown only when neither does.

Still open: everything about real Claude Desktop behaviour, and the size of a full
Microsoft 365 entry against the 4,096-byte limit. The proof run settles both.

## Objective

The portal owns connectors and stores each one exactly as the app reads it. The gateway
stores the list and hands it to each person's launcher. The launcher writes it into the
app and knows no connector by name. The first release makes Microsoft 365 in Claude
Desktop work this way, so a connector an admin sets once stays set for the whole team,
where today a person has to type it into Claude Desktop again after every launch.

The guiding document's "In three sentences" and "The experience we are building" are the
full statement. This file does not repeat them.

## The launcher's role

**What it does.** It reads the org's device policy, checks every custom connector entry
again with the shared refusal rule, and writes each one that passes into the app, as
given. For Claude Desktop that means one more item in `managedMcpServers` in the
launcher's own configuration entry, which is rewritten whole at every launch.

**What it deliberately does not do.**

- It keeps no list of Claude Desktop's built-in servers. It does not know that
  `microsoft365` exists. A built-in is any Claude Desktop blob that has `server`. Which
  built-ins are offered is the portal's decision alone.
- It does not read, fill in, default or reorder any field of an entry (tenant, client ID,
  scope, cloud, tool policy). They pass through untouched.
- It adds nothing to the policy document (`policy/launcher-policy.json`) for a connector
  of this kind. Slack stays as it is there; nothing new joins it.
- It holds no tokens and makes no request for a connector.

**Why.** Every connector added the Slack way costs a gateway deploy and a launcher
release. If the launcher only writes and refuses, the next connector is a portal-only
change. The launcher still runs the refusal rule itself because the gateway is not the
only place a document can come from (an org's own proxy, a hand-edited cache file), and
the machine that would be harmed is the last check. That reasoning is unchanged from
1.6.0.

**A word that now means two things.** In this code "built-in" already means the
connectors Consus defines in the policy document (Slack): `Check.built_in` in
`src-tauri/src/custom.rs`, `connector_kinds`, `connector_desktop_names` in
`src-tauri/src/policy.rs`. The guiding document uses "built-in" for a Claude Desktop
server that has `server` and no url. They are different things. This file says "Consus
connector" for the first and "built-in" for the second. The build should pick names that
keep them apart.

## What needs to change, and why

### (a) The shared fixture, `policy/connector-cases.json`

**Today.** 59 cases and a header of constants. This repository owns the file; the
gateway's tests carry a copy and the portal vendors it. Two suites here run it:
`every_shared_connector_case_is_kept_or_refused` in `src-tauri/src/settings.rs` (every
case, and the reason code of each refusal) and `the_fixture_header_equals_the_constants`
in `src-tauri/src/custom.rs` (each header key it names against a constant; it does not
walk the header, so a new key needs its own assertion). The release workflow publishes it as
`consus-launcher-connector-cases-<version>.json` (`.github/workflows/release.yml`).

Cases that matter here, already in the file:

- `name_with_space`: "my server" with a `claude_code` blob, refused `connector_name`. It
  stays correct under the new rule, because the entry has a Claude Code part.
- `claude_desktop_blob_only`, `claude_desktop_without_transport`: a Desktop-only remote
  server is accepted, with or without `transport`.
- `unknown_blob_keys_pass_through`: keys the rule does not name are accepted.
- `url_missing`: a blob without `url` is refused `connector_url`. Only a `claude_code`
  blob is tested; it stays correct.

**What must become true.** Both rule changes are written here first, then followed by
all three suites.

1. A Claude Desktop blob may be a built-in. A blob that has `server`:
   - must not carry `url`, `transport` or `type`;
   - skips the url check;
   - still passes the command, secret, `${`, size and depth checks;
   - is refused in a `claude_code` blob.
2. A custom entry with no Claude Code part may have single spaces in its name. Still at
   most 64 characters, still unique without regard to case, still not a Consus
   connector's name.

New cases the guiding document asks for: built-in accepted; built-in with `url` refused;
built-in with `transport` refused; built-in with a secret refused; `server` in a Claude
Code blob refused; a spaced name accepted for a Desktop-only entry; a spaced name
refused when the entry has a Claude Code part.

**Why here first.** It is the one place the three implementations agree. A rule that is
only in prose drifts.

**What the fixture pins, because nothing else does.** All of it is settled in the guiding
document's "The exact rules, for the fixture":

- the name pattern for a Desktop-only entry: words joined by one ordinary space, no space
  at either end, no two in a row, no tab, 64 characters at most;
- what "has a Claude Code part" means: the key is there and is not `null`. So
  `claude_code: null` leaves the entry Desktop-only, and `claude_code: 7` does not;
- the `server` value: 1 to 64 letters, numbers, `-` or `_`. Empty, a number or `null` is
  a refused built-in, not a remote server with a missing url;
- one reason code, `connector_server`, for a bad value, a built-in with `url` or a
  transport key, and `server` in a Claude Code blob;
- the order: the `server` check runs straight after the command keys and before both
  transport checks, so a Claude Code blob with `server` and no `type` is refused
  `connector_server`, and a built-in with `url` and a secret is refused
  `connector_server` too;
- the header gains `server_key`. The patterns are pinned by cases, not header fields.

Every accepted built-in case uses `server: "microsoft365"`, because the portal runs the
fixture through the function that refuses a value it has no form for.

### (b) The validators, `src-tauri/src/custom.rs`

**Today.**

- `check_blob(blob, client)` runs, in this order: object, size, depth, command keys,
  transport values, required transport key (`type` for Claude Code), secret keys, `${`,
  then `url` through `connector_url`. The url check is last and has no exception, so a
  blob without `url` is always refused (`connector_url`).
- `valid_name(name)` is 1 to 64 of ASCII letters, digits, `_` and `-`.
- `Check::entry` checks the name first, records it as taken, and only then looks at
  which blobs are present.

**What must become true.**

- `check_blob` has a built-in branch, straight after the command keys: `server` present
  means the Desktop client, a valid value, no `url` and no transport key, or the blob is
  refused `connector_server`. The secret and `${` checks still run; the transport and
  url checks do not. A blob without `server` is checked exactly as before.
- The name rule depends on whether the entry has a Claude Code part. `Check::entry`
  still refuses a bad name first, but it now looks for the Claude Code key before it
  picks which name rule applies. It needs only whether the part is present, not whether
  it is valid.

**Why.** A built-in has no url by definition, and Claude Desktop drops a built-in entry
that also carries `url`, `transport` or `command`. The name has to keep its spaces
because Claude Desktop ties a person's sign-in to the name it shows, and the guiding
document expects that keeping "Microsoft 365" means nobody signs in again.

**`valid_name` has a second caller, and it must stay strict.** `Record::load` uses
`valid_name` to drop any name in `connectors.json` that is not a safe server name,
because those names go on a `claude mcp remove` command line, which on a Windows npm
install runs through `cmd.exe`. The test `a_record_name_that_is_not_a_server_name_is_dropped`
asserts that `"a b"` is dropped. So there are two rules, as the guiding document now
says: the strict one (`valid_name`, unchanged) for anything that is a Claude Code key,
and a wider one beside it for a Desktop-only entry's name.

### (c) Everything that assumes a custom server has a url

A search of `src-tauri/src` and `src/main.ts` for every use of a custom server found
these. No other place reads `url` from a custom blob.

| Where | Today | With a built-in |
|---|---|---|
| `check_blob`, `custom.rs` | `url` required | covered in (b) |
| `CustomServer::host`, `custom.rs` | the host of the first blob's `url`, else `""` | returns `""` |
| `due`, `custom.rs` (the one-time notice) | "Your organization added the connector {name} ({host})." | "…the connector Microsoft 365 ()." |
| `Loaded::view`, `settings.rs` (the Keys tab's Connectors row) | "{name} ({host})" | "Microsoft 365 ()" |
| `seen_hash`, `blob_hash`, `canonical`, `custom.rs` | hash of `[name, claude_code, claude_desktop]` | no assumption about `url`; works as is |
| `add_args`, `claude_code.rs` | reads `url` from a Claude Code blob | never sees a Desktop blob |

**What must become true.** The notice and the Keys tab show a built-in's `server` value
where a remote server shows its host. Nothing may show empty brackets.

**Why.** The notice exists so a server pushed from a stolen portal session does not
arrive silently. It has to say something a person can recognise. The guiding document
names the `server` value for this.

**A mixed entry shows the url's host.** An entry may have a remote `claude_code` blob and
a built-in `claude_desktop` blob at once (the rule allows it when the name has no space).
Decided: the notice and the Keys tab show the url's host when either part has a url, and
the `server` value only when neither does. One consequence to know: `due` builds one
message and puts it on both tiles, so the Claude Desktop tile of a mixed entry shows the
Claude Code server's host and does not mention the built-in it wrote. A change to the
Desktop part still shows "changed" once.

The hashes need no change. A changed scope or tenant changes the Desktop blob, so
`seen_hash` changes and the person is told "changed" once. That is the existing
behaviour and it is the right one here.

### (d) Writing the entry into Claude Desktop's config

**Today.** `write_claude_desktop` in `src-tauri/src/config.rs` builds the launcher's
configuration entry from the policy document and sets `managedMcpServers` to the Consus
connectors (Slack) followed by `desktop_entry()` of each custom server.
`CustomServer::desktop_entry` in `custom.rs` clones the `claude_desktop` blob and sets
its `name` to the entry's name, replacing any `name` the blob had. Nothing else in the
blob is read or changed. The file is rewritten whole at each launch. Under the Claude
Desktop lock the launcher writes nothing.

**What must become true.** Nothing in the write path needs a new rule. A built-in blob
that passes the check goes through the same two functions and comes out as
`{"name": "Microsoft 365", "server": "microsoft365", …}` with no `url`, `transport` or
`command`, which is the shape Claude Desktop accepts. What the build owes here is a
test that shows it, in the style of `custom_servers_follow_the_built_in_ones`.

**Why it matters anyway.** The customer-visible promise ("set once, stays set") is
exactly this path: the entry is in the document, so the whole-file rewrite that removes
a hand-typed connector today puts this one back at every launch.

**How `name` is set, and what the portal must know.** The entry's top-level `name` is
the one Claude Desktop shows. A `name` inside the blob is allowed, checked by nobody,
and always replaced (tests `a_name_in_the_blob_is_replaced_by_the_entrys`,
`a_name_in_the_desktop_blob_is_allowed_and_not_checked`). So when the portal imports an
export and keeps the blob "as exported", the name that reaches the app is the entry's
name, not the exported blob's. The two must be the same for the sign-in to survive.
That is the portal's job; the launcher does not compare them.

### (e) The release

**Today.** The version is in five places that the release workflow refuses to build
without agreement on: `package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`,
`src-tauri/Cargo.toml` and `launcher` in `policy/launcher-policy.json` (README, "To cut
a release"; `policy.rs` has a test that the last equals the crate version). Each release
publishes the policy document and both fixtures as release files. The portal vendors
them and then moves its `LAUNCHER_VERSION` and `latest.json`.

**What must become true.**

- Version 1.8.0 in all five places, committed with the updated `Cargo.lock` (the
  workflow's check does not cover it).
- The policy document changes in its `launcher` value only. Nothing about a custom
  connector is in it today and nothing should be added. If the build finds it wants to
  add something there, that is a sign the launcher is starting to know a connector.
- The documents customers audit say what the launcher now writes. `README.md` ("Your
  organization's own connectors") says "remote servers only" and `SECURITY.md` says
  "remote `http` servers only"; both say "an `https://` URL". After this release a
  Desktop entry may be a built-in with no url. `SECURITY.md` also says a skipped
  connector gets "a log line that names it", which is not so for a bad name.
  `CHANGELOG.md` gets the 1.8.0 section in a person's words.
- The older docs that state the old rule get a pointer here, as
  `docs/device-policy-connectors.md` points to the 1.6.0 file today.

**Why.** The portal can only offer the form once a launcher that writes the entry
exists and the update banner can point at it.

## What I found in the code

Each item below is from the guiding document's "To confirm before or during the build".
"Read" means settled by reading the code at v1.7.0. No test was run and nothing was
built for this file.

### Old launchers (settled, by reading)

A 1.7.0 launcher skips that one entry and writes everything else. Nothing larger fails.

- `settings::check_document` hands `connectors` to `checked_connectors`, which calls
  `check_connectors`. That loops over the entries one at a time. An entry that fails
  goes on a `refused` list and the loop carries on. Each refusal is one log line
  ("device policy: connector not used (…)"), naming the code and never a value. It names
  the entry too, except when the name itself is what failed: then the line reads "a
  custom connector without a valid name" and cannot be tied to an entry.
- "Microsoft 365" fails `valid_name` in 1.7.0, so the entry is refused `connector_name`
  before its blob is looked at, with the log line that names nothing. A built-in under a
  name with no space would be refused `connector_url` instead, and that line does name
  it. Either way it is the same skip.
- The connectors branch of `check_document` never sets `outdated`, so a refused entry
  cannot hide the tools behind "update the launcher". Only an unknown `level` or
  `tools` value does that.
- Slack and every other custom server in the same document are kept and written. The
  test `connectors_come_from_the_document_entry_by_entry` shows the same for a kind the
  launcher does not know.
- If the refused entry was the only connector, `checked_connectors` returns nothing and
  the Keys tab shows no connectors from the portal. No error is shown to the person.
- A refused entry still counts toward the 16 custom entries. Its name is recorded as
  taken only when it was refused for a blob: an entry refused for its name records
  nothing. Neither matters for one Microsoft 365 entry.

Two consequences worth stating to the customer. A person on 1.7.0 gets no notice and no
connector, silently: the only trace is a log line, and for "Microsoft 365" that line
does not say which connector. The portal tells the admin instead (decided, above). And
they still lose a hand-typed
connector at each launch, because the whole-entry rewrite is unchanged.

**After the update, no new read is needed.** The cache file holds the document exactly
as the gateway sent it (`Cached.document`), and the check runs each time it is loaded.
So a 1.8.0 launcher accepts the entry from the cached copy even if the gateway answers
"not changed".

### Names with spaces (settled for the launcher, by reading; lock files are the portal's)

Every place a custom server's name is used:

| Place | Spaced name safe? |
|---|---|
| `connectors.json`, `seen` map (a JSON key) | Yes |
| `connectors.json`, `claude_code` map | Never reaches it: only entries with a `claude_code` blob are recorded |
| `seen_hash` | Yes (the name is a JSON string in the hashed text) |
| The notice text and the Keys tab row | Yes (plain text) |
| Log lines for a refusal | Yes for single spaces; the name is printed as is |
| `managedMcpServers[].name` in Claude Desktop's file | Yes as JSON. Whether the app is happy is not something the code can show |
| `claude mcp add` / `add-json` / `remove` arguments | Never reaches them (below) |
| Claude Code deny rules (`mcp__<server>__<tool>`) | Not written for custom servers |
| Any file or folder name | None is built from a server name; the record is always `connectors.json` |

So a spaced name breaks nothing in the launcher, provided it never becomes a Claude Code
key. Two things keep that true today and must still be true after the change: the rule
(a spaced name only when there is no Claude Code part) and `Record::load` dropping any
recorded name that fails the strict rule.

The guiding document also lists the two lock files (the macOS profile and the Windows
script). Those are rendered by the portal and are not in this repository.

### Does a Desktop-only entry touch the Claude Code path? (settled, by reading)

No. `claude_code::connector_servers` builds the wanted list from entries that have a
`claude_code` blob and skips the rest. `owned_names` in
`src-tauri/src/tools/connectors.rs` is built from the Consus connectors' keys, the
record's `claude_code` names and the wanted list. A Desktop-only entry is in none of
them, so it is never named in a `claude mcp` run, never written to the record's
`claude_code` map, and never compared with Claude Code's state file. `mcp_plan` and
`add_args` never see it.

One edge: an org that changes an existing entry from "has a Claude Code part" to
"Desktop only" under the same name. The name is still in the record, so the launcher
removes it from Claude Code at the next launch, as it should. The name had to pass the
strict rule to get into the record, so that remove is safe.

### The launcher writing a built-in (half settled)

By reading: a built-in blob that passes the check is written by `desktop_entry` and
`write_claude_desktop` unchanged apart from `name`. The launcher adds no `url`,
`transport` or `command` to a custom entry anywhere.

Not settled: that Claude Desktop accepts it when the launcher is the one that wrote it,
keeps the sign-in across launches, and treats a name with a space as the same connector.
Only a real run shows that. It is step 3 and 4 of the guiding document's "Prove it".

### The blob's own `name`, and the export's extra fields (settled, by reading)

- `desktop_entry` replaces the blob's `name` with the entry's. See (d).
- `check_blob` only looks at the keys the rule names. `oauth.mode`, `asSelection`,
  `tenantId`, `clientId`, `scope`, `azureCloud`, `continuousAccessEvaluation`,
  `sensitivityLabels` and `toolPolicy` are none of them, so they pass and are written
  back exactly as given. The fixture case `unknown_blob_keys_pass_through` already
  holds this for a remote blob.
- Three existing checks do apply to those fields and could surprise someone:
  - the secret check matches `headers` and `clientSecret` as a key at any depth, without
    regard to case, `_` or `-`. A `toolPolicy` that names a tool called `headers` would
    be refused. Unlikely, but real;
  - the blob is at most 4096 bytes as compact JSON. A long `scope`, a long `toolPolicy`
    and a list of sensitivity labels all count toward it. Nobody has measured a full
    entry;
  - no string may contain `${`.

Not settled: that those extra fields are harmless to Claude Desktop when written back.
The code only shows they arrive unchanged.

### Places that assume a url (settled for the launcher, by reading)

The table in (c) is the full list for this repository. The portal's server card, app
pages and lock-file output are the portal's to check.

### The export's two layouts (not the launcher's to settle)

The launcher writes the flat `managedMcpServers` list. Confirmed in
`write_claude_desktop`. It never reads an export. That the nested and flat layouts hold
the same server entries needs a real round trip.

### Items this repository cannot speak to

The permission list for the Access picker, tool names in tool policy, the app-wide
Microsoft settings, and government clouds. One note on the app-wide settings: if they
ever become "a separate field on the document", they would be keys in the launcher's
Claude Desktop entry outside `managedMcpServers`. The test
`the_entry_is_exactly_the_policy_documents_keys` in `config.rs` holds that entry to the
policy document's keys, so that would be a policy document change and a launcher
release, not a pass-through.

## Dependencies and order

**What the other two repositories need from the launcher.**

- The fixture, first. The gateway and the portal build their rule changes against it,
  so it has to be agreed and available before their work, well before the launcher is
  released. In practice the fixture and the launcher's rule merge to `main` together as
  an early step, and the other two copy the fixture from there.
- A released 1.8.0, with the policy document and the fixture as release files, for the
  portal to vendor before it offers the form.

**What the launcher needs from them.**

- From the gateway: that it accepts and serves the new entries. The launcher can be
  built and tested without it (its tests feed documents directly), but the real-machine
  check needs a deployed gateway that stores a built-in entry.
- From the portal: nothing to build against. To prove the release, an entry saved by
  the real form or import.

**Order.** The guiding document ships gateway, then launcher 1.8.0, then portal. That
order is safe from the launcher's side in both directions: a 1.7.0 launcher skips an
entry it does not know (above), and a 1.8.0 launcher against a gateway that has not
changed simply never receives one.

## Open questions and risks

**Questions, all closed.** The eight questions this file first raised are answered in
"Decided after this was written" at the top and written into sections (a) to (c): the
name pattern, the two name rules, where the name check sits, the reason code, the
`server` value, the fixture header, what a mixed entry shows, and the scan for stored
blobs that have both `url` and `server` (the gateway's, before its deploy).

**Where the code and the guiding document do not line up.**

- "The launcher … knows no connector by name" and "keeps no list of built-in names."
  True for Claude Desktop's built-ins after this release. But the launcher does know
  Slack by name, through the policy document, and `Check.built_in` is a list of names a
  custom entry may not take. The guiding document says Slack stays as it is for now, so
  this is wording and not a conflict. It is why this file avoids "built-in" for Slack.
- The guiding document says the fixture is written first and also that the launcher
  ships second. The fixture cannot land alone: the launcher's suite runs every case, so
  new cases fail until the rule changes. So the fixture and the rule merge together,
  early, and the release follows later. See Dependencies.

**Risks.**

- **Everything about real Claude Desktop behaviour is unverified here**: that it takes a
  built-in the launcher wrote, that the sign-in survives the launch-time rewrite, that
  the name with a space is treated as the same connector a person set up by hand, and
  that `oauth.mode` and `asSelection` are harmless when written back.
- **The 4096-byte limit** against a full Microsoft 365 entry. The limit is kept; the
  size of a real entry is still unmeasured.
- **Silent skip on old launchers.** Accepted. The person sees nothing, and the update
  banner is off by default on machines IT manages, so the portal's message to the admin
  is the prompt.
- **Locked machines.** Under the Claude Desktop lock the launcher writes nothing, so the
  connector reaches those machines only when IT pushes the portal's lock file again.
  That is existing behaviour, already in the README, and it applies here unchanged.

## Out of scope

From the guiding document:

- Hosting MCP servers, holding tokens, or proxying connector traffic.
- Anthropic's hosted Microsoft 365 connector as a form.
- Microsoft 365 in Claude Code, ChatGPT, VS Code or Pi. The built-in exists only in
  Claude Desktop, and the rule refuses `server` in a Claude Code blob.
- The customer editing the parts of the Claude Desktop configuration that are Consus's.
- Changes to either firewall.

And for the launcher in this release:

- Moving Slack off the policy document and onto a stored entry. It needs a blocked-tools
  list beside the entry first (the guiding document's "Decisions still open").
- Local command servers and servers that carry a secret. Still refused.
- Per-group connectors, and Claude Desktop fetching its configuration without the
  launcher.
- Reading or importing a Claude Desktop export. That is the portal's.
