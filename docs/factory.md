# Factory: a sixth tool

*How the launcher opens Factory's `droid` CLI against the Consus gateway, and what the
policy document must carry so the Consus portal can render Factory's lock. Drafted
2026-11-11 from a measured run (droid 0.245.1 and Factory.app 0.201.0 on macOS, every
request logged through a TLS proxy, a canary string in each prompt); revised the same day
after review (the Bearer question, `allowedBaseUrls`, org-level models measured, the id
rule, the defaults rule, the helper plumbing, the open policy call); revised 2026-11-12
(opt-in, macOS only, the lock carries the default-model keys, the per-command sandbox,
the version check, the home-override test before code). Counterpart:
`consus-key-portal/specs/factory.md`. The gateway needs no change. The Consus guide for
Factory (`gov_open_router/docs/integrations/factory.md`) is updated with the release.*

## What changes, in three sentences

Factory becomes the sixth tile, opt-in and macOS only: an admin turns it on in the
portal, and the launcher then writes `droid`'s settings into an isolated profile at
`~/.factory-consus-gateway`, pointed at the Consus models the key has, with the key read
through the launcher's helper, and opens `droid` in Terminal from `~/Consus`, as it does
Pi. Factory's enforcement keys (no web search, no image generation, Factory's models
refused, custom models only at the Consus endpoint, the default models) are read from
org-level settings, so the launcher also writes an org-level file into the profile and
names it in `droid`'s environment, and the portal renders the same keys into the
machine-wide file IT pushes. A user's prompts and responses then go only to the gateway;
what Factory still receives is the sign-in, feature flags, and per-turn token counts,
which Eric accepted (2026-11-11).

## What was measured (2026-11-11)

The bar: no prompt or response content reaches Factory. Airgap Mode (an enterprise build;
the public binary also honors `FACTORY_AIRGAP_ENABLED`, undocumented) is not used.

| Configuration | Plain prompt | `-m claude-opus-5-5` (a Factory model) | Prompt asking for a web search |
|---|---|---|---|
| Stock `~/.factory` | Default model is Factory's `gpt-6-sol`: content to Factory | Content to Factory | Query to `api.factory.ai/api/tools/web-search` |
| User settings hardened (below) | Consus only | Hidden from the picker, but still sent to Factory when forced | Query to Factory |
| Plus the org-level file | Consus only | Refused locally: "Model blocked by organization policy" | WebSearch tool removed; FetchUrl (a local fetch) remains |

With both files, every request carrying the canary went to `api.consus.io`. Requests that
still went to Factory, none carrying content: a WorkOS token refresh, `/api/cli/whoami`,
`/api/feature-flags`, `/api/connectors/tools/revision`, `/api/billing/limits`, and one
`POST prem.factory.ai/api/llm/custom/usage` per turn with the model id, base URL, token
counts, and session and message ids. With the org-level file present, droid skips its own
`GET /api/organization/managed-settings`.

Settled by measurement after review, the same day:

- **Both of Factory's request formats authenticate.** The gateway takes the key from
  `x-api-key` or `Authorization: Bearer` (`gov_open_router/src/authorizer.py:23`), and
  the WAF's `require-api-key` rule (`infrastructure/aws/waf.tf`) passes either header; a
  `chat/completions` call with Bearer and the helper's key answered. So `provider:
  "openai"` and `"generic-chat-completion-api"`, which send Bearer, work. The comment in
  the portal's `pi.ts` saying Bearer is a 403 is stale and is fixed in the portal PR.
- **A user's own custom model elsewhere is refused.** With
  `modelPolicy.allowedBaseUrls: ["https://api.consus.io"]` in the org-level file, a
  user-added entry with `baseUrl: https://api.openai.com/v1` is listed and then refused:
  "Model blocked by organization policy". The matcher (read from the binary) compares the
  exact origin and treats an entry whose path is `/` as any path, so the one entry
  `[endpoint]` admits both the bare endpoint (Claude) and `<endpoint>/v1` (the rest), and
  a look-alike host such as `api.consus.io.example.com` is refused.
- **Org-level `customModels` are honored and merge by id.** An org-level entry with the
  same `custom:consus-claude-opus-5-5` id as the user's replaced its display name and
  answered. So the launcher's user file and the portal's lock coexist on one machine.
  Measured through the local-path source; the machine-wide path itself is check 0 below.

Two features upload content by design and have no switch: Automations (their prompts, to
`/api/automations/sync`) and `/bug` (session logs). `builtInToolAutonomyOverrides` takes
only `low`, `medium`, `high`, and no org key turns Automations off, so there is no
technical control; the guide says so plainly and tells users not to use them.

## What Factory reads, and from where

- **User settings**, `<home>/.factory/settings.json`. Honored there: `customModels`
  (with an explicit `id`, so ids are stable; without one Factory derives
  `custom:<display-name-with-dashes>-0`), `cloudSessionSync`, `cloudConfigSync`,
  `cloudItemPull`, `telemetry.enabled`, `sessionEmbeddingChoice`, `modelFavorites`. A
  user-level `modelPolicy` only hides Factory's models.
- **Org settings**, one source, the first that exists: the machine-wide file
  (`/Library/Application Support/Factory/settings.json`, `/etc/factory/settings.json`,
  `C:\Program Files\Factory\settings.json`), then the file named by
  `FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH` (undocumented; measured), then a URL, then
  Factory's API. Org-only keys: `webSearchDisabled`, `imageGenerationEnabled`,
  `modelPolicy` as enforcement (`allowAllFactoryModels`, `allowCustomModels`,
  `allowedBaseUrls`), `disableAutoUpdate`, `hostedSessionEmbeddingsEnabled`,
  `advancedAnalyticsEnabled`, `wikiCloudSync`, `telemetry` as the sink. Org-level
  `customModels` merge with the user's by id, org entry winning.
- **Default models**, in the org schema: `sessionDefaultSettings.model` and
  `.specModeModel`, `compactionModel`, `subagentModelSettings`, `missionOrchestratorModel`,
  `missionModelSettings`. The user file can set them too; the org's value wins. Because
  they are org-schema keys, the lock carries them, a locked machine starts on the org's
  Consus default, and the launcher still writes nothing there (check 0 confirms).
- **Environment.** `FACTORY_HOME_OVERRIDE` replaces the home directory (Factory appends
  `.factory`; undocumented; measured 2026-11-12, check 0a).
  `FACTORY_DROID_AUTO_UPDATE_ENABLED=false` (documented). `FACTORY_API_KEY` is Factory's
  own credential, not ours.
- **Sandbox.** `sandbox.enabled`, `sandbox.mode`, `sandbox.network.allowedDomains`
  (applies to FetchUrl and shell commands); macOS and Linux only, not Windows.
  `sandbox.mode: "per-command"` sandboxes the agent's commands only, so the key helper
  and droid's own requests run outside it; the launcher pins it so a web-access-off
  sandbox with no allowed host cannot block the key or the gateway.
- **Credential.** `apiKeyHelper` per custom model is a command string Factory runs
  through a shell (the guide's form is `sh ~/.consus/helper.sh --tool factory`), so
  arguments work and no shim is needed; `apiKeyHelperTtlMs` is how often it re-runs.
  Provider by maker: `anthropic` with the bare endpoint for Claude (`x-api-key`),
  `openai` with `<endpoint>/v1` for GPT models that take the Responses API, and
  `generic-chat-completion-api` with `<endpoint>/v1` for the rest (Grok, GPT OSS); the
  last two send Bearer, which the gateway and the WAF accept.
- **The desktop app** reads `~/.factory` and the machine-wide file only. The env
  variables do not reach it, so the tile opens `droid`; the portal's lock is what reaches
  the app.

## Decisions

- **Opt-in.** Factory is not in the default tool set; an admin turns it on in the portal
  (or lists `factory` in `Tools`). Every Factory user signs in to Factory with their own
  identity, which a CUI or ITAR org should choose to allow. This is one default constant
  in two places, the launcher's default tool list in `settings.rs` and the portal's
  `fromDocument(null)`, not a per-level rule: an absent `Tools` now means "every tool but
  Factory" rather than "every tool". The README's `Tools` row reads "Default: all but
  Factory".
- **macOS only in v1.** Windows follows in its own release once three things are settled:
  droid's Windows install path, the machine-wide helper path for the lock, and the gap
  that FetchUrl and shell commands have no sandbox there. Its README row is then written
  the way Pi's is ("not restricted" where it is not). Until then the tile on Windows is
  hidden, the policy entry's lock has no `win` path, and the portal offers no Windows
  package for Factory (its spec).
- **Isolated profile** at `~/.factory-consus-gateway`, through `FACTORY_HOME_OVERRIDE`
  (Eric, 2026-11-11). The files land one level deeper than the other profiles, at
  `~/.factory-consus-gateway/.factory/`, because the variable replaces the home
  directory. Sign out removes the launcher's keys and leaves the directory, as for Pi.
- **Two files per launch.** `settings.json` carries the models, the defaults and the
  user-level switches; `consus-org-settings.json` in the same directory carries the
  org-only keys and the defaults, and the Terminal script sets
  `FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH` to it. Without the second file the tile is
  not safe on its own (the table above). If the variable stops working, the version
  check below refuses the tile, and the fallback is greying it until IT pushes the
  machine-wide lock.
- **A version check guards the undocumented variable.** Once per droid binary (its real
  path and file signature, which an update changes; a stat, not a `droid --version` call
  on every launch), the launcher runs `droid exec --list-tools --output-format json` with
  the tile's environment, timed out, and caches a pass or a refusal for that binary.
  WebSearch or GenerateImage in the list is a positive (measured with 0.245.1: without the
  org-level file droid lists both, with it neither): the org-level file is not being read,
  so the tile refuses to open and says why. No answer (timeout, a non-zero exit, output it cannot read) is "couldn't
  tell": the tile opens with a notice, and the check runs again on the next launch. Only a
  positive WebSearch refuses. The command calls Factory's API, so it is tested on a
  machine with no Factory login (check 2).
- **Factory's machine-wide file is the lock**, and when it exists the tile says "Managed
  by your organization" and the launcher writes nothing, as for Claude Code. The lock
  carries the default-model keys, so a locked machine starts on the org's Consus default
  without the launcher's user file. The portal renders it from this repository's policy
  document. It is a push-once file like the other locks: adding a model in the portal
  means IT pushes it again. Per-user sync of a tool's lock is the control-plane north
  star's business, not this release's.
- **Metadata to Factory is accepted.** The README's "What still leaves the machine" cell
  names it: sign-in, feature flags, token counts per turn. The tile is not zero-traffic
  the way Claude Code is, and the README says so.
- **Model ids**: strip the level suffix (`claude-opus-5-5:itar` becomes
  `claude-opus-5-5`), then dots to dashes, prefixed `custom:consus-`, so
  `custom:consus-claude-opus-5-5` and `custom:consus-gpt-5-4`. One level per launch, so
  no two models share an id; a display-name change never changes the id a user or a
  script picked. The portal uses the same rule, so a user who ran the launcher first
  keeps the same ids when the lock arrives (they merge by id).
- **Which model fills each default**, every rule falling back to the main model, so an org
  with one model sets every key to it:
  - main (`sessionDefaultSettings.model`): the newest Opus at the level, else what ChatGPT
    would start on, else the first model, as Pi does;
  - `specModeModel`, `missionOrchestratorModel`, `missionModelSettings.workerModel` and
    `.validationWorkerModel`, `subagentModelSettings.heavyModel`: main;
  - `compactionModel`: main. The compacting model must hold the conversation it
    compacts, and main is Factory's own default (`compactionModel: same`);
  - `subagentModelSettings.mediumModel`: the first Sonnet in preference order (what
    `claude_models` sorts by), else main;
  - `subagentModelSettings.lightModel`: the first Haiku in that order, else main.
- **Limits** per model: `maxContextLimit` from the gateway's `context_window` and
  `maxOutputTokens` from `max_output_tokens`, as `pi.rs` does. A row with neither is left
  out, as Pi leaves out embeddings.

## The policy document

A `factory` entry in `policy/launcher-policy.json`, schema 1, no new fields:

- `settings`: `cloudSessionSync`, `cloudConfigSync`, `cloudItemPull` false;
  `telemetry.enabled` false; `sessionEmbeddingChoice.provider` `"off"`.
- `slot_keys`: `customModels: models`, and the default-model keys filled from the
  `model` slot (in both the launcher's files and the lock). The per-model shape (provider
  by maker, base URL, `apiKeyHelper` from the `helper_shell` slot, `apiKeyHelperTtlMs`,
  limits from the gateway) is the writers' business, stated identically in both specs; a
  `note` on the `models` slot points here.
- `by_web_access.off`: `sandbox.enabled` true, `sandbox.mode` `"per-command"`,
  `sandbox.network.allowedDomains` empty. `on`: `sandbox.enabled` false.
- `lock_only.required`: `webSearchDisabled` true, `imageGenerationEnabled` false,
  `modelPolicy.allowAllFactoryModels` false, `modelPolicy.allowCustomModels` true,
  `modelPolicy.allowedBaseUrls` `["$endpoint"]`, `disableAutoUpdate` true,
  `hostedSessionEmbeddingsEnabled` false, `advancedAnalyticsEnabled` false,
  `wikiCloudSync` false. `allowedBaseUrls` is the one value that is a slot: the document
  carries the shape, and both renderers fill `$endpoint` the way connector entries fill
  `$slack_client_id` (`policy.rs` `fill`, the portal's `fillEntry`). No new `LockOnly`
  field. The launcher's org-level file carries the same keys, so the writer reads them
  from here too.
- `lock`: `kind: "settings-file"`, `paths.mac` `/Library/Application Support/Factory/settings.json`.
  No `win` path until the Windows release.
- `readme`: telemetry "Off: telemetry, cloud session and config sync, auto-update";
  web search "Removed by the org-level file"; web fetch "None / any host (Factory's
  sandbox)"; shell "None / any host (Factory's sandbox, macOS)";
  leaves machine "Sign-in, feature flags, and token counts per turn to Factory";
  lock "Factory's machine-wide `settings.json` from the portal".

The README table regenerates from this entry (`cargo test` prints it when it drifts).

## What changes where

Step one, before the Factory arm: `tools.rs` is 1,457 lines, so the terminal-script arms
move into `tools/terminal.rs` first, as a change of its own, to keep it under the soft
1,500 target.

| File | Change |
|---|---|
| `policy/launcher-policy.json` | The `factory` entry above |
| `src-tauri/src/factory.rs` (new, ~350 lines with tests) | `profile_dir`, `write_config`, `remove_config`, `no_models`, modeled on `pi.rs`. Owns only the `customModels` entries whose id starts with `custom:consus-` and the keys it writes; everything else in the file stays. Writes `consus-org-settings.json` from the filled `lock_only.required` and the default-model keys. Temp file and rename, both files read before either is written. The version check: run, time out, parse, cache by droid path and version |
| `src-tauri/src/policy.rs` | Accessors `factory_static`, `factory_variant`, `factory_org_keys` (the last runs `lock_only.required` through `fill` for the endpoint) |
| `src-tauri/src/tools.rs` | `Tool::Factory` in the enum and `TOOLS`; `key` `"factory"`, `name` `"Factory"`; `app_spec` None; `terminal_process` `"droid"`; `no_models`; `remove_config`; `FACTORY_HELPER_NAME` (`factory-key-helper`, a link to the binary like the others, written by `config::write_helper`); `FACTORY_SCRUB` (prefix `FACTORY`); `org_policy` true when the machine-wide file exists; the version check before launch; hidden on Windows. `cli(app, "droid")` already finds `~/.local/bin/droid` |
| `src-tauri/src/tools/terminal.rs` (new in step one) | The Factory arm exporting `FACTORY_HOME_OVERRIDE`, `FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH`, `FACTORY_DROID_AUTO_UPDATE_ENABLED=false`, PATH with droid's folder |
| `src-tauri/src/settings.rs:49` | `("factory", "factory")` in `TOOL_IDS`, its length 5 to 6; the default tool list is every tool but Factory (the `None` case of `Settings.tools` no longer means every tool) |
| `src/main.ts`, `index.html` | The tile, `"factory"` in `TOOLS`, vendor link `https://factory.ai`, "Not installed" copy, the "couldn't tell" notice and the refusal copy |
| `src-tauri/assets/factory.svg` | Factory's mark, when Eric supplies it; until then the tile wears Terminal's icon, like Claude Code |
| `README.md`, `CHANGELOG.md`, version | Tools table row (macOS only), `Tools` key list and "Default: all but Factory", the generated lockdown table, the metadata sentence, the Automations and `/bug` warning with "no technical control", and that turning Factory off removes the launcher's files but not the root-owned lock, which IT removes |

Tests that pin five tools and must become six: `policy.rs` (`ids`, the per-tool loop),
`tools.rs` (`off` list), `settings.rs` fixtures that use `"pi"` as the unknown-free case.

## Tests

- `factory.rs`: writes the trimmed model list for the key and level; provider by maker
  and base URL per provider; the id rule including the stripped level; the defaults rule
  for a full set, a Claude-only set, and one model; both files, the org file carrying the
  default-model keys; the org file carries `allowedBaseUrls` equal to the endpoint;
  `sandbox.mode` is `per-command` when web access is off; merge keeps a user's other
  `customModels` and settings; `remove_config` leaves them; an unreadable file changes
  nothing; no models at the level is the error "No ITAR models for Factory are available
  to this key." The version check: WebSearch in the output refuses; a list without it
  passes; a timeout, a non-zero exit and unreadable output are "couldn't tell"; a cached
  result is reused for the same version and dropped for a new one.
- `policy.rs`: the entry parses; the filled `required` carries `allowedBaseUrls` equal to
  `[endpoint]` and no string anywhere starts with `$`; the README table includes the row;
  the org-level keys are the lock's `required` set, no more.
- `settings.rs`: an absent `Tools` allows every tool but Factory; a `Tools` naming
  `factory` allows it.
- `tools.rs`: the Terminal script for Factory carries the three variables and no key.

## Verify on a real machine

0. **Done 2026-11-12** (droid 0.245.1, macOS, every request through a logging proxy, a
   canary in each prompt). The machine-wide file, written by hand with the 12 Consus
   ITAR models, the `required` keys, `allowedBaseUrls`, the default-model keys and the
   web-off sandbox, `sudo cp`'d to `/Library/Application Support/Factory/settings.json`,
   with `~/.factory/settings.json` stripped to stock:
   - the 12 models are listed from the lock; a plain `droid exec` used
     `custom:consus-claude-opus-5-5` (the lock's default, not Factory's `gpt-6-sol`), and
     the canary went only to `api.consus.io`;
   - GPT-5.4 answered through `/v1/responses` and Grok 4.6 through
     `/v1/chat/completions`, so `[endpoint]` admits both formats;
   - `droid exec -m claude-opus-5-5` and a user-added model at `api.openai.com` were both
     "Model blocked by organization policy", with no request leaving;
   - `--list-tools` had no WebSearch and no GenerateImage; a prompt asking for a web
     search got "I don't have a WebSearch tool";
   - under the per-command sandbox with no allowed host, a shell `curl` and a FetchUrl
     were both refused ("network access denied to example.com") while the key helper and
     the model request worked;
   - with the lock present droid still fetches `/api/organization/managed-settings`
     from Factory (the tier refresh); the file stays the source.
   Still to do: the Factory desktop app's picker (Eric), and a rerun of the GPT line
   whenever the supported droid version moves.

   0a. **Done 2026-11-12.** `FACTORY_HOME_OVERRIDE=<dir>` created `<dir>/.factory/` with
   droid's full layout, left `~/.factory` untouched, still read the machine-wide lock,
   and raised no daemon conflict with the daemon from `~/.factory`. The override home was
   **not signed in to Factory**: no WorkOS call, 401 on org settings, connectors and
   billing, and the per-turn usage report failed; the prompt still answered through
   Consus. So with the lock (or the launcher's org-level file) supplying the models, a
   launcher profile may need no Factory account at all. Open: whether interactive
   `droid` shows a sign-in screen in that state, and whether the org-level file through
   `FACTORY_ORG_MANAGED_SETTINGS_LOCAL_PATH` behaves the same without a login (the
   earlier env-var runs were signed in). Both are check 2.
1. **Done 2026-11-12** (droid 0.245.1, macOS, mitmproxy, a canary in each prompt), against
   the two files `factory.rs` wrote for this key's 12 ITAR models (not the machine-wide
   lock), droid started with the Terminal script's environment (the `FACTORY_*` scrub and
   the three variables, checked by running the script with `env` in place of droid):
   - a plain `droid exec` answered on `custom:consus-claude-opus-5-5`, the canary only in
     `POST api.consus.io/v1/messages`; Factory received billing limits, the connector
     list, feature flags and the per-turn usage report, none carrying the canary;
   - GPT-5.4 answered through `/v1/responses`, Grok 4.6 and GPT OSS 120B through
     `/v1/chat/completions`;
   - `-m claude-opus-5-5` and a user-added model at `api.openai.com` both exited 1 with
     "Model blocked by organization policy", nothing sent;
   - web access off: a shell `curl` got "Sandbox: network access was denied to
     example.com" and FetchUrl "Sandbox policy rejected this inner tool call", while the
     key helper and the model worked; web access on: the same `curl` left the sandbox;
   - the version check (`cargo test -- --ignored with_the_real_droid`) passes with the
     launcher's org-level file and refuses without it.
2. Interactive `droid` in a profile with no Factory login: whether it opens to a sign-in
   screen or to a session on the Consus default. If a session, the guide's "sign in to
   Factory" step becomes optional. Also the version check's `droid exec --list-tools`
   there: it either answers or times out into "couldn't tell", and never hangs the tile.

## Release

Build order: checks 0 and 0a are done; the policy entry, then the release. The release after
the one carrying Slack connectors (1.4.0, PR 70): 1.5.0. The policy asset
`consus-launcher-policy-1.5.0.json` is what the portal vendors; its spec lists the rest.
Older launchers skip the `factory` id in a device-policy document, so a portal that
turns Factory on does not break them.

## Out of scope

Windows, until its own release. The Factory desktop app as a tile (it ignores the env
variables; the lock reaches it). Airgap Mode. Factory's own `FACTORY_API_KEY`.
Automations and `/bug`, which have no control and which the guide tells users to avoid.
Linux. Per-user sync of the lock.
