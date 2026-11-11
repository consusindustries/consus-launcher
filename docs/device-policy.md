# Device policy: org settings from the Consus portal

*How the launcher reads the org's settings from Consus as well as from device management.
Drafted 2026-11-11; revised the same day after review (fail closed on `level`, `endpoint`
and an all-unknown `tools`; 404 and 401/403 keep the cache; the merge spelled out; a tool
with no models greyed out, not hidden; the cache tied to the key). The document and its
routes are defined by the gateway (`gov_open_router/specs/device-policy.md`); the admin
side by the portal (`consus-key-portal/specs/device-policy.md`). This file is the
launcher's half. It follows the handoff's guiding rule: the launcher stores one key,
writes each tool's config, and launches the tool. This adds one read, and one rule the
read makes necessary (a tool with no models is greyed out).*

## What changes, in three sentences

Today the launcher learns the org's settings only from a file IT pushes with device
management. From now on it also asks Consus, with the key it already holds, for the
settings the org's admin chose in the portal, so a change there reaches every signed-in
launcher the next time it opens, with no file. Device management still wins where it
sets a value, and the launcher says which values came from where.

## What the system already covers

- **The key, the host, the client.** `portal::validate_key` already calls `GET
  {endpoint}/v1/models` with `x-api-key` at startup and on connect. The new read is the
  same host, the same key, the same client and timeout.
- **The settings model.** `settings::Raw` already holds the five values (`EndpointURL`,
  `ComplianceLevel`, `Tools`, `OrgName`, `WebAccess`) and `settings::parse` validates
  them. The document is one more source; how the sources combine is restructured (The
  merge).
- **Removing a turned-off tool's settings.** `tools::remove_turned_off` already strips
  what the launcher wrote for a tool the org's settings leave out. It runs the same way
  for a tool the portal leaves out.
- **A dead key.** `discard_dead_key` already forgets the key and its metadata when
  `validate_key` gets a 401 or 403, and `revoke_key` (sign out) calls it. The cache joins
  what they remove. `validate_key` stays the only place a key is declared dead.
- **Endpoint validation.** `settings::normalize_endpoint` already rejects anything that is
  not an https URL to a plain host. It runs on the document's `endpoint` too.
- **The no-models messages.** Each writer already refuses to configure a tool the key has
  no models for ("No Claude ITAR models are available to this key.", `config.rs:120`,
  `claude_code.rs:215`, `chatgpt.rs:142` (ChatGPT and Codex), `pi.rs:83`). Greying out a
  tile shows the same message before the click instead of after.

## Decisions

- **Precedence, per field: device management, then the document, then settings set on
  this computer, then the default.** Each of the five values is decided on its own. A
  profile that sets only `EndpointURL` pins only that; the other four follow the portal.
  `UpdateNotice` and `managed` stay tied to device management, as now.
- **The document is read at startup and when the window gains focus, at most once a
  minute.** Not before each tool opens: a network call there would delay the click, and a
  focus refresh gets the same result (the person opened the launcher to click something).
  `launch_tool` reads the settings once and passes that snapshot down to the tool's
  writer, so a launch never mixes values from before and after a read. The gateway spec
  says the same ("at startup and when the window gains focus").
- **The read goes to the endpoint in effect.** That is the merged `EndpointURL`: device
  management if set, else the cached document's `endpoint`, else this computer's
  `EndpointURL`, else `api.consus.io`. `validate_key` uses the same value. The document's
  `endpoint`, when it changes, applies to the next launch and the next read, for every
  tool. It is re-checked with `normalize_endpoint` first; a value that fails is never
  followed (below). No value a customer admin can set through the portal reaches this
  field; only Consus sets it, which is why following it is safe.
- **A document field the launcher cannot use fails closed.** Device management keeps its
  rule: an invalid value is an error and hides every tool, because IT can fix the file. A
  document the gateway already validated can still carry a value this launcher version
  does not know; the person cannot fix that from the portal, and falling back to a default
  would loosen what the admin chose. So, per field:

  | Field | When this launcher cannot use it |
  |---|---|
  | `level` | An error: every tool hidden and no tool's settings removed (failed settings change nothing, as today), message "Update the Consus Launcher to use your organization's settings." Falling back to `itar` would configure tools at a level the admin did not choose. |
  | `endpoint` | The endpoint the read was sent to is kept, and one log line. Falling back to `api.consus.io` would quietly bypass an organization's logging proxy. |
  | `tools`, every id unknown | The same error and message as `level`: every tool hidden, nothing removed. An empty list instead would make `remove_turned_off` strip every tool's settings. A list of only unknown ids is a newer tool, not a typo (the gateway checked the ids' shape), so it must not widen to every tool. |
  | `tools`, some ids unknown | The unknown ids are skipped, the known ones apply. |
  | `org_name` | Dropped, one log line; the header shows "Consus". |
  | `web_access` | Dropped, one log line; off is the fallback, so it cannot fail open. |

- **Last good copy on disk**, tied to the key it was read with. So a launch with no
  network still has the org's settings, and a different key never inherits them.
- **Grey out a tool with no models.** The portal stores tools as the admin's intent, so a
  person can be given a tool their key has no models for; the old settings file filtered
  those out. A greyed tile with the reason is what a person, or IT on a call, can act on;
  a tile that vanished is not.
- **Say where a value came from.** A machine still holding a pushed settings profile
  should explain itself.
- **No daemon, no polling, no new host, no enforcement.** Unchanged.

## The read

`portal::fetch_device_policy(key, endpoint, etag) -> Result<Fetched, String>`:
`GET {endpoint}/v1/me/device-policy`, headers `x-api-key`, `accept: application/json`,
and `If-None-Match` when a cached ETag exists. Same `reqwest` client settings as
`validate_key` (`USER_AGENT`, 15 s). The mapping from status and body to the result is a
pure function, so it is tested without a server.

| Response | Result |
|---|---|
| 200 `{"device_policy": {...}}` | `Fetched::Document(doc, etag)`; written to the cache |
| 200 `{"device_policy": null}` | `Fetched::None`; the cache is removed (the org cleared it) |
| 304 | `Fetched::Unchanged`; the cache stands |
| 401, 403 | `Err`; the cache stands, one log line. The key is left alone: `validate_key` decides whether it is dead |
| 404 | `Err`; the cache stands. A gateway without the route yet, or an organization's logging proxy that does not forward it |
| 402, 5xx, network error | `Err`; the cache stands and is used |

The document's fields map onto `Raw` as device management's keys do: `tools` →
`Tools`, `web_access` → `WebAccess`, `level` → `ComplianceLevel`, `org_name` → `OrgName`,
`endpoint` → `EndpointURL`. `updated_at` is kept for display. Nothing else in the body is
read.

**Cache:** `device-policy.json` in the launcher's own folder (`~/Library/Application
Support/io.consus.launcher/`, `%APPDATA%\io.consus.launcher\` on Windows), holding the
document as received (before the per-field checks, so a newer launcher re-reads it with
its own rules, even on a 304), its ETag, `fetched_from` (the endpoint the read was sent to,
which a bad document `endpoint` falls back to on later starts too), and `key_hash`: the
first 16 hex characters of the SHA-256 of the key it was read with (`sha2` is already a
dependency). Loaded with the settings on first use, so the startup `remove_turned_off`
already applies it, and before the first fetch. A `key_hash` that does not match the keychain's key means the cache is ignored and
removed, so a pasted key from another org starts from device management and defaults
until its own read. No secret is in the file.

## The merge

`settings.rs` keeps `Raw`, `parse`, and the device-management readers. What changes is
how their results combine:

- **Two `Raw`s.** `read_from` today walks all four macOS sources (or three registry keys)
  first match wins, into one `Raw` with one `managed` bool. Instead the readers fill two
  `Raw`s: `mdm` from the managed sources (`/Library/Managed Preferences/<user>/`,
  `/Library/Managed Preferences/`; `HKLM\…\Policies`, `HKCU\…\Policies`) and `local` from
  the rest (`~/Library/Preferences/`, `/Library/Preferences/`; `HKCU\Software\Consus\
  Launcher`). Each is first-match-wins within itself, as today. `/Library/Preferences`
  stays non-managed, as today. Both are read once at startup.
- **`merge(mdm, document, local) -> (Raw, Sources)`**: for each of the five fields, the
  first of `mdm`, `document`, `local` that set it, else none (the default). `UpdateNotice`
  comes from `mdm` then `local`, never the document. `managed` is derived from `mdm`
  alone: true when it set any field.
- **`Sources`** records, per field, `Mdm`, `Portal`, `Local`, or `Default`.
- **State.** The frozen `OnceLock` becomes `Loaded { mdm, local, base, doc: Option<Cached>,
  settings, endpoint, sources, … }` behind a lock, where `base` is device management and this
  computer alone (today's result, whose error stands whatever the document says).
  `settings::current()` is unchanged for every caller. `settings::store(Cached)` saves a new
  document and `settings::forget()` drops it; both re-run the checks, the merge, and `parse`.
- **`settings::endpoint()`** returns the merged endpoint on its own, so `validate_key` and
  the read still have a host when another field failed (an unknown level hides the tools
  but leaves the key connected and the focus refresh running, so the fix arrives without a
  restart). A device-management endpoint that fails is still an error here, as today.

**Errors, by source.** A device-management or local field that fails `parse` keeps
today's error, with today's prefix ("Your organization's launcher settings are invalid:
…"). A document field is checked on its own before the merge, per the table in Decisions:
`org_name` and `web_access` are removed from the document with one log line; a bad
`endpoint` is replaced by the cache's `fetched_from`; an unknown `level`, or a
`tools` list with no id this launcher knows, makes `settings` the error "Update the
Consus Launcher to use your organization's settings.", shown as is (no prefix, no
"Contact your IT admin."). Device management overrides the document field by field, so a
field device management sets is never checked from the document.

## After a read

The UI, after `validate_key` at startup and after each focus refresh that returned a
document or null (anything but 304 or an error):

- Reloads the settings view (org name, sources, error) and re-runs `detect_tools`, so a
  tool the admin turned off disappears.
- Calls `remove_turned_off`, so that tool's written settings go, as device management's
  do today (for Claude Desktop and ChatGPT, at a refresh when the app is not open).
- A changed level, web access, or endpoint applies to the next launch.

## Grey out a tool with no models

`detect_tools` takes the models the UI already holds (`currentModels`, the `/v1/models`
listing from `validate_key`) and adds `no_models: Option<String>` to `ToolStatus`: the
writer's own message when the key has no models for the tool at the level in effect, else
none.

| Tool | No models when | Message |
|---|---|---|
| Claude Desktop, Claude Code | `models::claude_models(models, target)` is empty | "No Claude ITAR models are available to this key." |
| ChatGPT, Codex | the GPT list `chatgpt::` builds from is empty | "No GPT ITAR models are available to this key." |
| Pi | `models::at_level(models, target)` is empty | "No ITAR models for Pi are available to this key." |

(The level in each message is `Target::tag()`, so it reads "FedRAMP High" and so on.) The
check is the writer's, moved earlier; the strings come from one function per writer, not
copies. A tile with `allowed` and `no_models` shows greyed out, its status line carries
the message, and clicking it does nothing. Greyed is not turned off: its written settings
stay, and it becomes usable at the next refresh where the key can call a model for it.
When the model list could not be read (a startup with no network), `no_models` is none for
every tool and the writer's error stands if someone opens one.

## Where a value came from

- **Header.** Under the org name, one line: "Settings from your IT" when any field came
  from device management; "Settings from the Consus portal" when any came from the
  document and none from device management; both, joined with "and", when mixed; nothing
  otherwise.
- **Keys tab.** A "Settings" block listing the five values, each with "Set by your IT",
  "From the Consus portal", "Set on this computer", or "Default". A portal value shows the
  document's `updated_at` once, under the block. This is where a person, or IT on a call,
  sees at a glance that a pushed profile is still pinning a machine.
- `SettingsView` gains `sources` (per field) and `portal_updated_at`.

## Sign out and a dead key

`discard_dead_key` also removes `device-policy.json`. After sign out the next start reads
device management, this computer's settings, and defaults only, and the next sign-in
fetches fresh. A pasted key reads with whatever key is in the keychain; the `key_hash`
check drops a cache read with another key.

## Documents customers audit

- **README, "What it connects to":** the fourth request. `GET .../v1/me/device-policy` on
  the same host as `/v1/models`, with the key, at start and when the window gains focus
  (at most once a minute), to read the settings the organization's admin set in the
  portal; it carries nothing about the person beyond the key. **"For IT admins: org
  settings":** one sentence that a value set here wins over the same value set in the
  portal, so a pushed settings profile pins those values until it is removed; and one for
  proxy operators: an organization's logging proxy must forward `GET
  /v1/me/device-policy` and pass `If-None-Match` and `ETag` through, or the launcher keeps
  the settings it last read.
- **SECURITY.md:** the request under "What the launcher does"; `device-policy.json` under
  "Files it writes"; sign out removes it. The sentence "nothing is fetched for them" about
  org settings goes. One sentence added: these settings shape what the launcher shows and
  are not enforced; the gateway decides what a key can call.
- **`docs/consus-launcher-handoff.md`:** section 6 lists the route; section 10's next item
  1 is done; this file is linked from section 2.
- **`policy/launcher-policy.json`:** unchanged. No lock key changes, so the portal's
  vendored copy and lock renderers are untouched.
- **CHANGELOG:** the entry, and the line IT needs: "If you pushed the Consus Launcher
  settings profile, remove it to get settings from the portal."

This repository is public: none of these documents, or this file, names a customer.

## Tests

`settings.rs`:
- `the_document_fills_what_device_management_left`: mdm sets `EndpointURL` only, the
  document sets all five; endpoint from mdm, the rest from the document; `sources` says so.
- `device_management_wins_per_field`: both set `Tools`; mdm's list applies.
- `the_document_wins_over_this_computer`: local and the document both set `WebAccess`;
  the document's applies; `sources` says `Portal`.
- `an_unknown_document_level_is_an_error`: the update message; every tool hidden;
  `endpoint()` still answers.
- `a_bad_document_endpoint_keeps_the_one_the_read_used`: the cache holds
  `https://proxy.a.example`, the new document `http://proxy.example`; the endpoint stays
  `https://proxy.a.example`, one log line.
- `an_unknown_document_tool_id_is_skipped`: `["claude-desktop", "future-tool"]` shows
  Claude Desktop only; `["future-tool"]` alone is the update error: every tool hidden, not
  every tool shown, and `remove_turned_off` removes nothing.
- `a_bad_document_org_name_is_dropped`: the other fields apply.
- `managed_and_update_notice_follow_device_management_only`: a document alone leaves
  `managed` false and the update notice on.
- `a_cache_for_another_key_is_removed`: a `key_hash` mismatch reads as no document and
  deletes the file.
- `nothing_set_means_defaults` and the existing tests: unchanged.

`portal.rs` (the response mapping):
- `a_304_keeps_the_cache`, `a_null_document_removes_it`, `a_200_writes_it_with_its_etag`,
  `a_401_or_403_keeps_the_cache_and_the_key`, `a_404_keeps_the_cache`, `a_5xx_keeps_it`.

`tools.rs`:
- `a_tool_with_no_models_is_greyed_not_turned_off`: `no_models` carries the writer's
  message, `allowed` is true, and `turned_off` does not list it.
- `no_model_list_greys_nothing`.

## Release

1. One PR, reviewed, CI green: the read, the merge, the cache, the greyed tile, the
   sources view, the documents above.
2. Real-machine check on macOS and Windows, against the deployed gateway: sign in, see
   "Settings from the Consus portal"; turn a tool off in the portal, refocus the launcher,
   the tile is gone and its settings removed; turn it back on, it returns; turn on a tool
   with no models at the level, see it greyed with its message; go offline, restart, the
   settings hold; sign out, `device-policy.json` is gone; push the settings profile, see
   "Set by your IT" on its fields and the portal change not apply to them.
3. Tag and publish (a minor version: new behavior, nothing removed), then the portal's
   `LAUNCHER_VERSION` and `latest.json` bump as usual.

## Out of scope

- Reading connectors, branding, or any field the document does not yet carry. Each is a
  gateway change first, then a `Raw` slot here.
- Per-group settings: the gateway resolves those when it has them; the read does not
  change.
- Any refresh other than startup and focus. No timer, no background process.
- Reclassifying `/Library/Preferences` (written with `sudo defaults write`) as managed.
  It stays "Set on this computer", as today.
- Claude Desktop's own `bootstrapUrl`. If Consus ever serves one, Claude Desktop reads it
  itself; the launcher is not involved.
