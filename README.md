# Consus Launcher

Sign in with your company account once. Click a tool, and it opens already configured for the Consus gateway.

Consus Launcher is a small open source desktop app for macOS and Windows. It writes each tool's configuration and launches it. It is a front door, not a gateway and not an enforcement agent.

## Signing in

The launcher has no accounts of its own. It signs you in through the Consus portal, and the portal signs you in through your organization's identity provider (Okta, Microsoft Entra ID, Google Workspace, or any SAML or OIDC provider your Consus admin has connected). Click **Sign in with Consus**, approve in the browser, done.

What that gets your organization:

- **Every request has a name on it.** The portal issues a key owned by the person who signed in, labelled with their computer, and expiring after 30 days. The launcher keeps it in the system keychain and hands it to each tool. Nobody copies a key, so there is no key to share, leak, or lose track of.
- **Admins see and revoke from one place.** Each key is listed in the portal under the person and computer it belongs to. Revoke it there and every tool on that computer stops at its next request. Leaving the organization, losing a laptop, and rotating on a schedule are all the same action.
- **Sign out means revoked.** Signing out in the launcher revokes the key in Consus before forgetting it, so it does not live on for the rest of its 30 days.
- **One key per computer.** Signing in again replaces and revokes the previous key. The same key serves the [Consus credential helper](https://docs.consus.io/credential-helper) for tools the launcher does not open, such as Factory, and the launcher adopts a key the helper already obtained.
- **The source is open; the identity is yours.** Anyone can read what the launcher does. What it cannot do is mint a credential: only your identity provider, through the Consus portal, can put a person behind a key, and only the Consus gateway decides what that key may reach.

Together with the gateway, which decides where every request goes and refuses what is outside the boundary, and the device-management settings below, which decide what each person's tools may do, this is the whole chain: your IdP decides who, the portal issues the credential, the gateway enforces, the launcher sets up the tools.

Organizations that have not connected an identity provider yet can paste a key from the portal instead ("Have an API key instead?" on the sign-in screen). A pasted key has no expiry of its own and is revoked in the portal, not by signing out.

## Tools

| Tool | How it opens | Where the launcher writes its settings |
|---|---|---|
| Claude Desktop | The app, in gateway mode | `~/Library/Application Support/Claude-3p/configLibrary/` |
| ChatGPT | The app, in Codex mode | `~/.codex/config.toml`, merged key by key |
| Claude Code | Terminal | `~/.claude-consus-gateway/` (its own profile) |
| Codex CLI | Terminal | `~/.codex-consus-gateway/` (its own profile) |
| Pi | Terminal | `~/.pi-consus-gateway/` (its own profile) |
| Factory (`droid`), macOS only, off unless your organization turns it on | Terminal | `~/.factory-consus-gateway/.factory/` (its own home) |

- **Your own setup is left alone.** Claude Code, Codex CLI, Pi, and Factory each get a separate profile folder, so `~/.claude`, `~/.pi`, and `~/.factory` are never touched and Codex CLI runs apart from `~/.codex`. ChatGPT has no such option: its settings are merged into `~/.codex/config.toml`, which your own `codex` also reads. Terminal tools start in an empty `~/Consus` folder.
- **Models:** each tool is set up with the ITAR models your key can use, from `GET /v1/models`. ChatGPT and Codex also get a model catalog built from ChatGPT's own Codex engine, so their pickers list the Consus models by name.
- **The key** (see "Signing in" above) lives in the macOS Keychain (Windows Credential Manager on Windows) and is never written to a file. Tools fetch it through the launcher itself (a keychain helper), or receive it in the environment of the process the launcher starts. The Keys tab shows who is signed in and when the key expires.
- **App windows:** on macOS a tool opens inside the launcher's glass area, then moves freely (not yet on Windows). Quitting the launcher quits the tools it started, never ones you opened yourself.
- **Claude and ChatGPT restart once.** If either app is already open when you click it, the launcher asks it to quit and reopens it in gateway mode, because both read their settings only at startup. On Windows, where closing these apps only hides them to the tray, the launcher ends them if they do not quit.
- **Sign out** revokes this computer's key in Consus, so it stops working everywhere, then removes the launcher's settings from each tool and keeps everything else, including your history. A pasted key can only be revoked in the portal, and the launcher says so. One working key per computer: signing in or pasting a key while a sign-in key is held revokes that one, and signing out of a key adopted from the credential helper signs the helper out too.
- A tool that is not installed links to its vendor. Consus never installs software.
- **Factory** is opt-in: every Factory user signs in to Factory with their own identity, so it shows only when your organization lists it (`factory` in `Tools`, or turned on in the Consus portal). The launcher writes two files into its own Factory home: `settings.json` (the Consus models, the default models, telemetry and cloud sync off, the sandbox) and `consus-org-settings.json`, which `droid` reads as organization settings and which removes web search and image generation, refuses Factory's own models, and admits custom models only at your endpoint. Before the first launch with each new `droid`, the launcher asks `droid` for its tool list: if web search is in it, that `droid` is not reading the organization file and the tile refuses to open; if `droid` gives no answer, the tile opens with a notice. Prompts and responses go only to Consus; Factory still receives your sign-in, its feature flags, and token counts for each turn. Two Factory features upload content and have no setting that turns them off, so there is no technical control for them: Automations (their prompts) and `/bug` (session logs). Do not use them. When IT pushes Factory's machine-wide `settings.json` (from the portal), the launcher writes nothing and opens `droid` under it, as for Claude Code. Turning Factory off removes the launcher's files, not that root-owned file, which IT removes.
- **Connectors.** When your organization's admin turns on a connector in the Consus portal (Slack today), the launcher sets it up in Claude Desktop and Claude Code, so each person only clicks Connect once in each. The connector runs where its vendor runs it (Slack's at `mcp.slack.com`) and talks to the laptop directly, never through Consus; what Claude reads from it goes to the model through the gateway like any prompt. Slack is read-only: the Slack app your organization registers is given read scopes only, which is the control, and the launcher also blocks Slack's write tools in both apps as a second layer (a best-effort list, reviewed each release). In Claude Desktop the launcher owns the connector list in its own configuration entry, so a connector added to that entry by hand is gone at the next launch (try a new connector with Claude opened directly). In Claude Code the launcher asks Claude Code to add or remove the server named `slack` (`claude mcp`, user scope, in the launcher's profile); that replaces a server you named `slack` yourself, and your other servers stay. Turned off in the portal, Slack is gone from both at each app's next launch; sign out removes it too. ChatGPT, Codex, and Pi get no connectors: Codex's sign-in redirect cannot be registered with a Slack app, and Pi's MCP support is an extension you install. Factory gets none in this release.

**On Windows:** Claude Desktop's settings go to `%LOCALAPPDATA%\Claude-3p`; ChatGPT's to `%USERPROFILE%\.codex\config.toml`; the terminal tools' profile folders and `Consus` folder are in `%USERPROFILE%`, and each opens in its own console window.

## What it does not do

- No proxy and no TLS interception, ever.
- No enforcement, monitoring, or config repair.
- No tray icon, no autostart, no background process.
- No local database; no analytics, telemetry, or crash reporting.
- No auto-update. New versions come from the [Releases](https://github.com/consusindustries/consus-launcher/releases) page.

## What it connects to

The launcher makes four kinds of network request, all to Consus. `GET https://api.consus.io/v1/models` (or the same path on your organization's endpoint, if one is set), when you connect and each time the launcher starts, to check the key and list your models; and `DELETE .../v1/me/key` on the same host when you sign out, to revoke it. `GET .../v1/me/device-policy` on the same host, with the key, when the launcher starts, when you connect, and when its window comes forward (at most once a minute), to read the settings your organization's admin chose in the Consus portal; it sends nothing about you beyond the key, and the launcher keeps the last answer for when it is offline. `POST https://portal.consus.io/api/cli/exchange` while you sign in, every two seconds until you approve in the browser; it carries a one-time secret and receives your key once. And `GET https://portal.consus.io/assets/launcher/latest.json`, a public file naming the newest release, so it can say when an update is out; it sends no key and nothing about you, and is off by default on machines IT manages (`UpdateNotice` below). The portal's sign-in page opens in your browser. (The Terminal installer downloads from GitHub; the app itself never does.)

The tools you open send their requests to `api.consus.io`, or to your organization's endpoint, with the settings the launcher writes. A connector your organization turned on (Slack) is the exception by design: Claude Desktop and Claude Code reach the connector's own server (`mcp.slack.com`, and Slack's sign-in page once) directly; the launcher itself makes no request for it. When it sets Slack up in Claude Code it runs `claude mcp` with the same switches as a session, which keep that run off Anthropic's network (checked through a logging proxy, Claude Code 2.1.322). What else a tool does is up to the tool. For Claude Code and Claude Desktop, the launcher's config turns off every connection to Anthropic that a setting can turn off: analytics, error and crash reports, feature flags, update checks, the WebFetch domain check, and the plugin marketplace download (Claude Code), and connector icons, the connector directory, the artifact preview (so artifacts do not render), the model catalog poll, and user-added plugin marketplaces and uploads (Claude Desktop). It also removes signing in to a Claude.ai account: `/login` and the launcher sign-in in Claude Code, and the Claude.ai option on Claude Desktop's start screen. One connection remains: at the first Cowork or Code session, Claude Desktop downloads its sandbox VM bundle and Claude CLI binary from `downloads.claude.ai`; only Anthropic's offline installer avoids that. Neither app updates itself, so install new versions yourself (`claude update` for Claude Code). If your organization's managed settings configure either app, the launcher writes nothing and these settings come from your organization. For ChatGPT and Codex, the launcher's config turns off analytics, feedback, and OpenTelemetry export, and Codex's update check; for Pi, install telemetry and the update check; for Factory, telemetry, cloud session and config sync, and auto-update, and Factory still receives sign-in, feature flags, and per-turn token counts (see Tools above). On its first run, Pi downloads `fd` and `ripgrep` from GitHub, because its search tools need them.

**Web access.** Web search is never offered: it is a tool that runs on the model provider's side, outside the compliance boundary, and the gateway refuses it. What the agent itself may reach from the machine is off by default and turned on for everyone with the `WebAccess` setting below. Off means: Claude Code has no WebFetch tool and its shell commands run in Claude Code's own sandbox with no host allowed; Factory's shell commands and FetchUrl run in Factory's sandbox with no host allowed; Claude Desktop has no Web Fetch and its sandbox reaches no host; the commands ChatGPT and Codex run have no network. On means each of those may reach any host your network allows. Where the setting does not reach: Claude Code's sandbox exists on macOS, and on Linux once `bubblewrap` and `socat` are installed, but not on Windows, so its shell commands are never restricted there; Pi has no sandbox at all, by its authors' design, so its shell commands are never restricted anywhere; and Factory's sandbox covers its shell commands and FetchUrl on macOS, where the launcher offers it. For those, and for the applications' own connections, the control is your network edge. Claude Code 2.1.219 or later refuses a blocked host outright; an older Claude Code asks the user instead, and before 2.1.216 also confines shell writes to the `~/Consus` folder.

Per tool, with `WebAccess` off (the default) and on:

<!-- lockdown-table:start (generated from policy/launcher-policy.json; `cargo test` prints the table when it drifts) -->
| | Inference | Vendor telemetry and updates | Web search | Web fetch, off / on | Agent shell network, off / on | What still leaves the machine | Lock it for the user |
|---|---|---|---|---|---|---|---|
| Claude Desktop | Gateway only | Off: crash and error reports, analytics, model catalog, connector icons, previews, auto-update | Never offered | None / any host | None / any host (its sandbox VM) | Sandbox VM bundle from `downloads.claude.ai` at the first Cowork or Code session | `com.anthropic.claudefordesktop` profile from the portal |
| Claude Code | Gateway only | Off: analytics, error reports, feature flags, update checks, plugin marketplace, WebFetch domain check, sign-in paths | Never offered | None / any host | None / any host on macOS, and on Linux with `bubblewrap` and `socat` (Claude Code's sandbox); not restricted on Windows | Nothing measured (2.1.307, through a logging proxy) | `managed-settings.json` from the portal, which can also carry `allowedProviders` and `allowManagedDomainsOnly` |
| ChatGPT | Gateway only | Off in the agent: analytics, feedback upload, OpenTelemetry export. The app around it keeps OpenAI's own update and product-analytics connections | Off in the file; the gateway strips it if turned back on | No such tool | None / any host (Codex's sandbox) | The app's own connections to OpenAI | `requirements.toml` from the portal |
| Codex CLI | Gateway only | Off: analytics, feedback, OpenTelemetry export, update check | Same as ChatGPT | No such tool | None / any host (Codex's sandbox) | Nothing the launcher knows of | `requirements.toml` from the portal |
| Pi | Gateway only | Off: install telemetry, update check | No such tool | No such tool | Not restricted: Pi has no sandbox | `fd` and `ripgrep` from GitHub on first run | Pi offers no lock; a container or the network edge |
| Factory | Gateway only | Off: telemetry, cloud session and config sync, auto-update | Removed by the org-level file | None / any host (Factory's sandbox) | None / any host (Factory's sandbox, macOS) | Sign-in, feature flags, and token counts per turn to Factory | Factory's machine-wide `settings.json` from the portal |
<!-- lockdown-table:end -->

## For IT admins: org settings

On macOS the launcher reads settings for the `io.consus.launcher` preferences domain. Push them in a configuration profile with your device management (Jamf, Intune, and so on); a value from a profile wins over the user's own preferences, which win over the machine's (`/Library/Preferences`). On Windows it reads the same names as registry values under `HKLM\SOFTWARE\Policies\Consus\Launcher` (what Intune and Group Policy write), then `HKCU\SOFTWARE\Policies\Consus\Launcher`, then the user's own `HKCU\Software\Consus\Launcher`; `Tools` can be a `REG_MULTI_SZ`. Every key is optional.

| Key | Type | What it does |
|---|---|---|
| `EndpointURL` | string | Where every tool sends its requests, for example your own logging proxy. Default `https://api.consus.io`. Must be `https://` (plain `http://` only to `localhost` or `127.0.0.1`). |
| `ComplianceLevel` | string | The compliance level every tool is set up for: `itar` (default), `fedramp-low`, `fedramp-moderate`, `fedramp-high`, `il2`, `il4`, or `il5`, each optionally with `+itar` (for example `fedramp-high+itar`). For CUI, use `fedramp-high`, as the Consus portal does. |
| `Tools` | array of strings | The tools shown: `claude-desktop`, `chatgpt-desktop`, `claude-code`, `codex-cli`, `pi`, `factory`. Default: all but Factory, which is shown only when listed, so to add it, list every tool you want (macOS only). When a tool is left out, the launcher removes the settings it wrote for that tool, as sign out does: when it starts, or, for a tool turned off in the Consus portal, when it next reads the portal's settings. If Claude or ChatGPT is open then, it tries again later. A tool the key has no models for is shown greyed out with the reason, and keeps its settings. |
| `OrgName` | string | Shown in the launcher's header. |
| `UpdateNotice` | boolean | Whether the launcher says when a newer version is out (it reads a public file on `portal.consus.io` and links to the release; it never updates itself). Default: on, except on a machine with settings from device management, where IT pushes updates. |
| `WebAccess` | `on` or `off` | Whether the agent in each tool may reach the web from the machine: Claude Code's WebFetch and shell sandbox, Claude Desktop's Web Fetch and sandbox, and the sandbox network of ChatGPT and Codex (the table above). Default `off`. Web search stays off either way. |

Connectors (Slack) have no key here: they come from the Consus portal only, and the device-management path for them is the lock files below, which carry the connector themselves.

The launcher reads these when it starts; restart it after a change here (a change in the Consus portal needs no restart). If a value is present but invalid or blank, or a settings file cannot be read, the launcher explains the problem and sets nothing up, rather than falling back to defaults. Hiding a tool here is a convenience, not a control: what a key can do is enforced by the Consus gateway.

**Settings from the Consus portal.** The same values, all but `UpdateNotice`, can also be set by your organization's admin on the Consus portal's Launcher page; the endpoint there is set by Consus at your organization's request. Each signed-in launcher reads them with its key when it starts and when its window comes forward (at most once a minute), and keeps the last copy for when it is offline. A value set here, through device management, wins over the same value in the portal, so a pushed settings profile pins its values until it is removed; the portal's values in turn win over a user's own preferences. The launcher's Keys tab shows where each value came from ("Set by your IT", "From the Consus portal", "Set on this computer", or "Default"). If your organization's endpoint is a logging proxy, it must forward `GET /v1/me/device-policy` and pass the `If-None-Match` and `ETag` headers through; otherwise the launcher keeps the settings it last read. A portal value this launcher version does not know, such as a newer compliance level or tool, hides the tools and asks the person to update the launcher, rather than falling back to a default.

**One policy document.** Every value the launcher writes into a tool, and every value only a lock may carry, is in one file in this repository, [`policy/launcher-policy.json`](policy/launcher-policy.json). The launcher embeds it; the Consus portal renders the same file into the lock files below for the launcher version it offers; the table above is generated from it. Read that file to know exactly what a locked-down tool looks like.

The launcher writes each tool's settings where the user can edit them. To make them a policy the user cannot change, push the tool's own managed settings as well: Claude Code's `managed-settings.json` (which can also carry `allowedProviders`, a key Claude Code reads only from managed settings, refusing every claude.ai and Console sign-in), Claude Desktop's `com.anthropic.claudefordesktop` profile, and Codex's `requirements.toml`. The Consus portal's deployment page generates all three; when one is present, the launcher leaves that tool's connection settings alone and labels the tool "Managed by your organization".

To try settings on one Mac without a profile:

```sh
defaults write io.consus.launcher Tools -array claude-code codex-cli
defaults write io.consus.launcher OrgName "Example Org"
defaults delete io.consus.launcher    # back to defaults
```

### Signing identity and permissions

From v0.2.3, the macOS app is signed with Developer ID by Consus Industries, Inc. To grant its permissions ahead of time, so people see no prompts, add a Privacy Preferences Policy Control (PPPC) payload to the profile you push. macOS honors PPPC only from device management, not from a profile installed by hand.

- **Bundle ID:** `io.consus.launcher`
- **Team ID:** `K4P2D65BQD`
- **Code requirement:** `identifier "io.consus.launcher" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = K4P2D65BQD`
- **Apple Events (Automation)**, allowed, to: `com.apple.Terminal` (terminal tools), `com.apple.systemevents` (window placement), `com.anthropic.claudefordesktop` and `com.openai.codex` (the ChatGPT app), to close and reopen them with new settings.
- **Accessibility**, allowed: to place app windows inside the launcher.

## Install

Requires macOS 12 or later on Apple silicon or Intel (tested on macOS 26), and a Consus API key from your Consus admin.

**From Terminal:**

```sh
curl -fsSL https://raw.githubusercontent.com/consusindustries/consus-launcher/main/install.sh | sh
```

[`install.sh`](install.sh) downloads the latest release from GitHub, checks the disk image against the release's `SHA256SUMS.txt`, installs Consus Launcher into `/Applications` (or `~/Applications`), and opens it. Nothing is installed unless the checksum matches. Set `CONSUS_LAUNCHER_VERSION` to pick a release (for example `v0.1.0`); to install from an internal mirror of the release files, set `CONSUS_LAUNCHER_BASE_URL` and `CONSUS_LAUNCHER_VERSION` together.

The checksum catches a corrupted or mismatched download. Who built it is proven by the signature: from v0.2.3, releases are signed by Consus Industries, Inc. (Apple Developer ID, team K4P2D65BQD) and notarized by Apple.

**On Windows:** download the `.msi` from [Releases](https://github.com/consusindustries/consus-launcher/releases) and run it. Most PCs (Intel and AMD) take `Consus-Launcher-<version>-windows-x64.msi`; `-windows-arm64.msi` is only for ARM PCs such as Snapdragon and Surface Pro X, and on any other PC it stops with "not supported by this processor type". From v1.0.0, the installer and the app are signed by Consus Industries, Inc. (Microsoft Artifact Signing). Earlier versions were unsigned, and SmartScreen warned on first run.

**From the download (macOS):**

1. Download the `.dmg` (drag to Applications) or the `.pkg` (installs to `/Applications`) from [Releases](https://github.com/consusindustries/consus-launcher/releases).
2. Optionally, check the download against `SHA256SUMS.txt` with `shasum -a 256 -c --ignore-missing SHA256SUMS.txt`.
3. Open Consus Launcher and sign in.

From v0.2.3, the `.dmg`, the `.pkg`, and the app are signed by Consus Industries, Inc. and notarized by Apple, so macOS opens a downloaded copy without a warning. (Earlier releases were not notarized; use v0.2.3 or later.)

On first use, macOS asks for:

- **Keychain access**, to read your key. Choose Always Allow.
- **Automation**, to open terminal tools in Terminal and to place windows.
- **Accessibility** (System Settings, Privacy and Security, Accessibility), to place app windows inside the launcher. Without it, apps still open, just not in place.

## Updating

- **Machines IT manages:** push the new `.pkg` or `.msi` the way the first one went out. The launcher reads its settings each time it starts and rewrites a tool's settings the next time it opens it, so nothing else needs to change.
- **Installed yourself:** the launcher says when a newer version is out (see [CHANGELOG.md](CHANGELOG.md) for what changed). Run the Terminal installer again, or download the new `.dmg`, `.pkg`, or `.msi` from [Releases](https://github.com/consusindustries/consus-launcher/releases). Your key stays in the keychain.

The launcher never updates itself.

## Uninstalling

1. **Sign out first** (Keys tab, Sign out). It revokes your key, removes the launcher's settings from every tool, and removes the key from the keychain. Without it, tools stay pointed at a key helper that is no longer there.
2. **Remove the app.** macOS: quit it and move `/Applications/Consus Launcher.app` to the Trash. Windows: Settings, Apps, Installed apps, Consus Launcher, Uninstall.
3. **Optional, everything else:** the profile folders that hold your tool history (`~/.claude-consus-gateway`, `~/.codex-consus-gateway`, `~/.pi-consus-gateway`, `~/.factory-consus-gateway`, under `%USERPROFILE%` on Windows), the `~/Consus` folder, and the launcher's own folder (`~/Library/Application Support/io.consus.launcher` and `~/Library/Caches/io.consus.launcher` on macOS, `%APPDATA%\io.consus.launcher` on Windows).

IT: also remove the settings profile (macOS) or the `HKLM\SOFTWARE\Policies\Consus\Launcher` key (Windows).

## Troubleshooting

**Slack: Connect fails with a redirect error.** The organization's Slack app must list exactly these redirect URLs: `http://localhost:53280/callback` (Claude Desktop) and `http://localhost:3118/callback` (Claude Code), with `localhost`, not `127.0.0.1`. The Slack app page has a separate Save button for redirect URLs.

**Slack: Connect fails at once.** Something else on the laptop is listening on port 53280 or 3118. Quit it and try again.

**Slack is missing in Claude Code.** Open Claude Code from the launcher again: the launcher adds it before the session starts and says on the tile if Claude Code could not. `/mcp` then shows `slack` as needing authentication until you authenticate once.

**Slack is missing on a machine IT locked.** The lock file carries the connector, not the launcher. IT pushes the lock again after setting the Slack client ID in the portal.

## Build from source

You need Rust (stable), Node 20, and the Xcode command line tools.

```sh
npm ci
npm run tauri dev      # run in development
npm run tauri build    # build the app and a .dmg locally
cargo test --manifest-path src-tauri/Cargo.toml
```

Dependencies are pinned to exact versions and `Cargo.lock` is committed. The UI is plain HTML, CSS, and TypeScript with no runtime packages beyond Tauri's own.

## Releases and SBOM

Release builds are made only in GitHub Actions ([`release.yml`](.github/workflows/release.yml)), never on a laptop, so the public workflow log is the provenance record for every installer. Each release carries:

- `Consus-Launcher-<version>-universal.dmg` and `.pkg` (macOS), and `Consus-Launcher-<version>-windows-x64.msi` and `-windows-arm64.msi` (Windows), signed as described under "Install"
- `consus-launcher-rust.cdx.json` and `consus-launcher-npm.cdx.json`: CycloneDX SBOMs for the Rust crates and the npm packages in the app
- `consus-launcher-policy-<version>.json`: the policy document, every setting this version writes (see "One policy document" under "For IT admins" above)
- `SHA256SUMS.txt`

Signing and notarization run in that same workflow once the Apple secrets are set; the secret names are listed at the top of `release.yml`. Run the workflow by hand (Actions, Release, Run workflow) to try signing before tagging.

To cut a release:

1. Set the new version with `npm version --no-git-tag-version <version>` (updates `package.json` and `package-lock.json`), then in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and `launcher` in `policy/launcher-policy.json`. Commit it with the updated `Cargo.lock`. The workflow refuses to build if any of the five differ.
2. Tag the commit `v<version>` and push the tag.
3. The workflow drafts a GitHub Release with the files above. Review it, then publish.

## License

[Apache-2.0](LICENSE)
