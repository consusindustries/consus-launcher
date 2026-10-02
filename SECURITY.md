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
- Two kinds of request. The key check: `GET https://api.consus.io/v1/models`, or the same path on the organization's endpoint when an IT admin set one (`EndpointURL`, see the README), when you connect or replace your key and each time the launcher starts, to check the key and list its models. It uses the system proxy settings and the system's trusted certificates.
- The update check: `GET https://portal.consus.io/assets/launcher/latest.json`, a public file naming the newest release, at start and twice a day. It sends no key and nothing about you. If a newer version is listed, the launcher shows a link to this project's GitHub releases; it never downloads or installs anything itself. Off by default on a managed machine (`UpdateNotice`, see the README).
- Org settings are read locally from the `io.consus.launcher` preferences (a device-management profile, or the user's own preferences); nothing is fetched for them.
- In your browser, not from the app: `portal.consus.io`, and a tool's vendor download page when you click a tool that is not installed.

**The key**
- Stored only in the macOS Keychain, or Windows Credential Manager. The launcher never writes it to a file.
- Tools get it through the launcher's own binary acting as a helper (Claude Desktop, Claude Code, ChatGPT, Codex, Pi), or in the environment of the process the launcher starts (ChatGPT on macOS, alongside the helper, and Codex's Terminal session). ChatGPT sends the helper's key as a bearer token and the environment's as `x-api-key`; on Windows, ChatGPT must be started through its package, which passes no environment, so it uses the helper alone.
- The keychain protects the key from other users and from files on disk, not from your own programs: any program running as you can run the helper or read a tool's environment and obtain the key.

**Files it writes**
- Each tool's settings, listed in the README: `~/Library/Application Support/Claude-3p/` (Claude Desktop, including a mode marker file and a one-time backup of its `_meta.json`), `~/.codex/config.toml` (ChatGPT), and the profile folders `~/.claude-consus-gateway`, `~/.codex-consus-gateway` (including a generated model catalog), and `~/.pi-consus-gateway`.
- `~/Consus`, an empty folder the terminal tools start in.
- Links to its own binary, used as the key helpers, in `~/Library/Application Support/io.consus.launcher/` (on Windows, hard links named `*-key-helper.exe` in `%APPDATA%\io.consus.launcher\`). The same folder holds ChatGPT's generated model catalog (`chatgpt-models.json`) and an empty `codex-probe` folder used when building it.
- An icon cache in `~/Library/Caches/io.consus.launcher/`.
- Sign out removes the launcher's settings from every tool and ChatGPT's catalog. When org settings leave a tool out, the launcher removes its settings from that tool the same way when it starts (for Claude or ChatGPT, at a start when the app is not open). Neither touches a file or library entry the launcher did not write. It leaves the profile folders (they hold your history), `~/Consus`, the helper links, the icon cache, and the Claude Desktop backup.
- On Windows the same files live under `%LOCALAPPDATA%\Claude-3p` (Claude Desktop) and `%USERPROFILE%` (the rest).

**Programs it runs**
- `osascript`, to open Terminal windows and to place app windows (macOS asks for Automation and Accessibility; see the README).
- Your login shell, once per tool, to find where a command-line tool is installed (`$SHELL -ilc "command -v <tool>"`).
- `codex debug models --bundled`, to build the Codex and ChatGPT model catalogs.
- `sips` and `defaults`, to read app icons; `ps` and `pgrep`, to track the tools it started.
- On Windows: `powershell` (to find Store apps and their processes), `explorer.exe` (to start ChatGPT through its package), `where.exe`, `tasklist`, and `taskkill`, to find, start, track, and close the tools it started. Closing a running Claude or ChatGPT before reopening it ends it if it only hides to the tray.

**Builds**
- Release builds are made only in GitHub Actions, with CycloneDX SBOMs and SHA-256 checksums attached to each release.
