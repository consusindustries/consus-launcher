# Security

Consus Launcher runs on your machine, holds a Consus API key in the system keychain, and writes configuration for other apps. We take reports about it seriously.

## Reporting a vulnerability

Please report privately, not in a public issue:

- Use GitHub's private reporting: the **Security** tab of this repository, then **Report a vulnerability**.

Include what you found, how to reproduce it, and the version (the release you installed, for example v0.1.0). We will acknowledge your report, keep you updated while we fix it, and credit you in the release notes if you would like.

Reports about the Consus gateway (`api.consus.io`) or portal (`portal.consus.io`) are welcome through the same channel.

## Supported versions

Fixes land in the latest release. Please check that the issue still reproduces there before reporting.

## What the launcher does, for reviewers

**Network**
- Four kinds of request. The key check: `GET https://api.consus.io/v1/models`, or the same path on the organization's endpoint when one is set (`EndpointURL`, see the README), when you connect or replace your key and each time the launcher starts, to check the key and list its models; and `DELETE .../v1/me/key` on the same host at sign out, to revoke the key. It uses the system proxy settings and the system's trusted certificates.
- Sign-in: `POST https://portal.consus.io/api/cli/exchange`, every two seconds while you sign in, with a one-time secret the launcher made (PKCE S256: the portal's page saw only its hash). The answer that carries your key arrives once, over TLS, and goes straight to the keychain. This is the Consus CLI login protocol the credential helper script also speaks.
- The update check: `GET https://portal.consus.io/assets/launcher/latest.json`, a public file naming the newest release, at start and twice a day. It sends no key and nothing about you. If a newer version is listed, the launcher shows a link to this project's GitHub releases; it never downloads or installs anything itself. Off by default on a managed machine (`UpdateNotice`, see the README).
- Org settings: read locally from the `io.consus.launcher` preferences (a device-management profile, or the user's own preferences), and from the Consus portal with `GET .../v1/me/device-policy` on the same host as the key check, with the key and the last answer's ETag, at start, on connect, and when the window comes forward (at most once a minute). It sends nothing but the key. Device management wins over the portal field by field. A value the launcher version does not know hides the tools rather than falling back to a default, and an endpoint that is not a plain https URL is never followed. These settings shape what the launcher shows and sets up; they are not enforced. The gateway decides what a key can call.
- In your browser, not from the app: `portal.consus.io`, a tool's vendor download page when you click a tool that is not installed, and this project's GitHub releases page when you click an update notice.

**The key**
- Stored only in the macOS Keychain, or Windows Credential Manager. The launcher never writes it to a file. A key the credential helper script left in `~/.consus/key` is read once, on first run, and used from the keychain after that; the launcher never writes there.
- Sign out revokes the key through the gateway (a sign-in key may revoke itself; a pasted key may not, and the launcher says to use the portal), then deletes it from the keychain.
- Tools get it through the launcher's own binary acting as a helper (Claude Desktop, Claude Code, ChatGPT, Codex, Pi), or in the environment of the process the launcher starts (ChatGPT on macOS, alongside the helper, and Codex's Terminal session). ChatGPT sends the helper's key as a bearer token and the environment's as `x-api-key`; on Windows, ChatGPT must be started through its package, which passes no environment, so it uses the helper alone.
- The keychain protects the key from other users and from files on disk, not from your own programs: any program running as you can run the helper or read a tool's environment and obtain the key.

**Files it writes**
- Each tool's settings, listed in the README: `~/Library/Application Support/Claude-3p/` (Claude Desktop, including a mode marker file and a one-time backup of its `_meta.json`), `~/.codex/config.toml` (ChatGPT), and the profile folders `~/.claude-consus-gateway`, `~/.codex-consus-gateway` (including a generated model catalog), and `~/.pi-consus-gateway`. In Claude Code's `settings.json` the launcher also owns two rules in `permissions.deny` (`WebSearch`, and `WebFetch` unless the org turns web access on) and these keys under `sandbox`, written with web access off and removed otherwise: `enabled`, `failIfUnavailable`, `allowUnsandboxedCommands`, `autoAllowBashIfSandboxed`, `filesystem.disabled`, `network.allowedDomains`, `network.strictAllowlist` (Claude Code's own shell sandbox, network part only, no host allowed); every other rule and key, including a user's other sandbox keys, stays.
- What those settings make the tools do on the network is in the README under "What it connects to". The claims there were checked against Claude Code 2.1.307 and Claude Desktop's published telemetry and egress reference on 2026-10-26.
- Connectors the organization turned on in the Consus portal (Slack): in Claude Desktop, a `managedMcpServers` list inside the launcher's own configuration entry; in Claude Code, the launcher owns the deny rules for Slack's write tools (`mcp__slack__<tool>`) in `settings.json`, and Claude Code's own user config in the launcher's profile holds the `slack` server, which the launcher never writes itself: it asks Claude Code to add or remove it through Claude Code's CLI (`claude mcp add-json` / `remove`, user scope), and reads that file only to see whether anything needs to change. Sign out removes all of it. The launcher never reads or writes either app's OAuth tokens; each app keeps those in its own store.
- `~/Consus`, an empty folder the terminal tools start in.
- Links to its own binary, used as the key helpers, in `~/Library/Application Support/io.consus.launcher/` (on Windows, hard links named `*-key-helper.exe` in `%APPDATA%\io.consus.launcher\`). The same folder holds ChatGPT's generated model catalog (`chatgpt-models.json`), an empty `codex-probe` folder used when building it, and `signin.json`: who signed in, the organization, the key's id, label and expiry, and how the key arrived; and `device-policy.json`: the last settings read from the Consus portal, the endpoint they were read from, their ETag, and the first 16 hex characters of the key's SHA-256, so another key never uses them. No secret is in either.
- An icon cache in `~/Library/Caches/io.consus.launcher/`.
- Sign out removes the launcher's settings from every tool, ChatGPT's catalog, and `device-policy.json`. When org settings leave a tool out, the launcher removes its settings from that tool the same way when it starts or reads new settings from the portal (for Claude or ChatGPT, when the app is not open). Neither touches a file or library entry the launcher did not write. It leaves the profile folders (they hold your history), `~/Consus`, the helper links, the icon cache, and the Claude Desktop backup.
- On Windows the same files live under `%LOCALAPPDATA%\Claude-3p` (Claude Desktop) and `%USERPROFILE%` (the rest).

**Programs it runs**
- `osascript`, to open Terminal windows and to place app windows (macOS asks for Automation and Accessibility; see the README).
- Your login shell, once per tool, to find where a command-line tool is installed (`$SHELL -ilc "command -v <tool>"`).
- `codex debug models --bundled`, to build the Codex and ChatGPT model catalogs.
- `claude mcp add-json` and `claude mcp remove`, in the launcher's Claude Code profile with the session's no-traffic switches and the launcher's own Claude environment scrubbed, only when the connectors Claude Code holds differ from what the organization chose (at a Claude Code launch, when Claude Code is turned off, and at sign out).
- `sips` and `defaults`, to read app icons; `ps` and `pgrep`, to track the tools it started.
- On Windows: `powershell` (to find Store apps and their processes), `explorer.exe` (to start ChatGPT through its package), `where.exe`, `tasklist`, and `taskkill`, to find, start, track, and close the tools it started. Closing a running Claude or ChatGPT before reopening it ends it if it only hides to the tray.

**Builds**
- Release builds are made only in GitHub Actions, with CycloneDX SBOMs and SHA-256 checksums attached to each release.
