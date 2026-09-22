# Consus Launcher: handoff

Written 2026-09-21. Wireframe: `docs/consus-launcher.html` (open it in a browser and click through). Screenshots in `docs/screenshots/`.

Guiding rule for everything below: this is the simplest application possible. It stores one key, writes four config files, launches four apps, and shows one list from the portal. Anything beyond that is out of scope unless Eric says otherwise.

## 1. What it is

A small open source desktop app for Mac and Windows. The user pastes a Consus API key once. After that, clicking a tool (Claude Desktop, ChatGPT, Claude Code, VS Code) opens it already configured for the Consus gateway. A Log tab shows the requests those tools make, metadata only.

It is a front door, not a gateway and not an enforcement agent. It writes configs and launches apps. Nothing else.

## 2. Decisions already made

- **App, not a daemon.** Open it, use it, close it. No tray process, no background service, nothing resident.
- **Consus enforces nothing.** If a user edits a config to bypass Consus, that is their IT's problem to detect. The launcher does not watch, reset, or block.
- **Consus installs nothing third-party.** A tool that is not installed shows "Not installed, get it" with a link to the vendor's download page. The launcher never downloads or installs another app.
- **Paste-a-key onboarding.** No OAuth, no browser handshake. The user creates a key at portal.consus.io, copies it, pastes it into the launcher. The launcher validates it by calling the models endpoint.
- **Key lives in the OS keychain.** macOS Keychain, Windows Credential Manager. Never written to disk in plain text by the launcher itself. Never displayed after entry; show only the last four characters.
- **One default key, optional per-tool override.** Per-tool keys are behind an "Advanced" toggle.
- **No local database.** Settings are one JSON file. Log is a live view, not stored.
- **Log is metadata only.** Host, time, size, tool, model, key suffix, latency. Never request or response content. v1 sources the log from the portal's request feed for the user's keys, not from a local proxy.
- **Self-hosted, open source.** Public repo, permissive license. Signed builds on GitHub Releases.
- **Tool list for v1:** Claude Desktop, Claude Code, VS Code (Copilot chat harness with `customendpoint`). ChatGPT is in the UI as a fourth tool but ships in v1.1, once Eric confirms how ChatGPT desktop is pointed at a custom endpoint; until then it shows "Coming soon" and does nothing. Pi is not in the launcher.
- **Templates are built in for v1.** One file per tool in `src-tauri/templates/`, with a placeholder for the model list. The launcher fills the list from `GET /v1/models` and writes the file. Later, when the portal exposes a templates endpoint for the Deployment page, the launcher prefers that and falls back to the built-in copies. Not a v1 concern.

## 3. Repo

Repo: `consus-launcher` on GitHub under the Consus org. Public. Apache-2.0 license. Eric creates the empty repo and adds `docs/`; Claude Code does everything else.

Layout after scaffolding:
```
consus-launcher/
  docs/                 handoff, wireframe, screenshots
  src/                  web UI: index.html, styles.css, main.ts
  src-tauri/            Rust core and Tauri config
    src/                keychain.rs, tools.rs, config.rs, portal.rs, main.rs
    tauri.conf.json     bundle id io.consus.launcher
    icons/
  .github/workflows/    build.yml (Mac + Windows), release.yml (signed builds on tag)
  README.md
  LICENSE
```
Keep the Rust side small enough to read in one sitting. Target: under 1,500 lines total. If a change pushes past that, stop and ask.

### Supply chain and network rules (customers review this)
- **Network:** the launcher talks to `portal.consus.io` and `api.consus.io` only. No other host, ever, including at build time for assets the app loads at runtime. Fonts (Archivo Black, JetBrains Mono, Space Grotesk; all open license) are bundled in the app, not loaded from Google Fonts. Remove the Google Fonts link when porting the wireframe.
- **Dependencies:** pin exact versions. Commit `Cargo.lock`. No web-side package manager dependencies at all; the UI is plain HTML, CSS, and TypeScript compiled with the bundler Tauri's template ships. If a convenience library seems necessary, ask first.
- **SBOM:** generate a CycloneDX SBOM (`cargo cyclonedx`) for every release and attach it to the GitHub Release alongside the installers.
- **Reproducible releases:** release binaries are built only in GitHub Actions, never on a laptop, so the public workflow log is the provenance record. Signing and notarization happen in that same run.
- **Third-party code review:** keep the dependency list short enough that a security team can read `Cargo.toml` in a minute and recognize every entry.

## 4. Stack

- **Tauri 2** (Rust core, system webview). Not Electron.
- **Frontend:** plain HTML, CSS, TypeScript. No framework required; the wireframe is close to final markup. Svelte is acceptable if it helps.
- **Keychain:** `keyring` crate.
- **Settings:** `tauri-plugin-store`, one file: `settings.json` in the app config dir.
- **Updater:** Tauri updater, releases signed with a Consus-controlled key committed as public key in the config.
- **Bundle identifier:** `io.consus.launcher`. Use it for the Tauri config, keychain service name, and config directory. Do not change it after the first signed release.
- **Signing:** macOS Developer ID Application + Installer with notarization (Apple enrollment in progress, see section 9); Windows via Azure Trusted Signing. Build must not fail when signing secrets are absent (local dev builds unsigned).

## 5. Screens and behavior

### Window
Frameless, resizable, cream/ink/orange design system (Archivo Black display, JetBrains Mono labels and code, Space Grotesk body). Left area is transparent "glass" showing the desktop; the launched app is a separate OS window behind it. Right side is a 300px panel. See wireframe for exact layout and copy.

### First run (not connected)
Panel shows one card, "Connect to Consus," with three steps:
1. Open the portal (button opens `https://portal.consus.io` in the default browser).
2. Create a key (copy instructions only).
3. Paste it here (masked input) and Connect.
On Connect: trim, check minimum length, then call `GET /v1/models` with the key. On success store the key in the keychain, store the returned user email and model list in memory, switch to connected. On failure show one plain error: empty, looks cut off, rejected by portal, or network. Errors clear when the input changes. (Real keys have no required prefix; there is no client-side format check beyond a basic length guard, the portal call is the actual validation.)

### Connected: Open tab
Status pill turns green "CONNECTED." List of four tools, each with an icon, name, subtitle, and a state:
- **Open:** installed and configured. Click writes/refreshes the config, then launches the app.
- **Running:** the launcher started it this session.
- **Get it:** not installed. Click opens the vendor download page in the browser. Nothing else.
Subtitle shows "Last used N min ago" when known (from portal request timestamps for this key), else the tool's description.
Footer note: signed in as `<email>`; every tool uses Consus; anything not installed links to the vendor; Consus never installs software.

### Connected: Keys tab
- Default key card: masked suffix, user email, model count, date added, Replace button (inline paste field with Save / Cancel / Open portal).
- "Advanced: one key per tool" toggle reveals one card per tool: Default or Own key, with "Use a different key" and "Use default."
- "Sign out and remove keys": deletes all keychain entries, removes the configs the launcher wrote, returns to first run.

### Connected: Log tab
Dark terminal-style panel. Header: pulsing dot, destinations count ("1 destination"), and live/idle. Body: newest-first monospace lines, one per request: time, host, size, tool, model, key suffix, latency. Lines animate in. Blinking cursor line at the bottom. Data source: poll the portal's request feed for this user's keys every 2 s while the Log tab is visible. Nothing persisted. Footer: "Log is metadata only: where, when, how big. Content is never read or stored on this machine."

### Revoked key
If any call returns 401/403 for a key, mark that key invalid, show a plain message ("This key was revoked. Paste a new one from the portal."), and open the paste field for that key. If it was the default key, return to first run.

## 6. Portal dependency

v1 needs exactly one portal call, and it already exists: **`GET /v1/models`** with the user's key in `x-api-key`. It validates the key and returns the models the key can use (id with regime suffix, context window, max output, tool calling, vision). The launcher drops those models into the built-in templates.

**Add if missing:** the key's owner email, either on that response or as `GET /v1/me` returning `{email, org, key_name}`. Until it exists, show the key's last four characters where the wireframe shows an email.

**Later, not v1:**
- `GET /v1/launcher/templates/{tool}?os=` so templates come from the portal (shared with the Deployment page).
- `GET /v1/launcher/requests?since=` returning request metadata for the calling key (timestamp, tool from a `X-Consus-Client` header the launcher sets in configs, model, bytes, latency; no content) to power the Log tab and "last used." Until it exists, the Log tab shows the wireframe's copy with "Coming soon" and the Open tab shows tool descriptions instead of "Last used."

Write specs for both to `docs/portal-endpoints.md` as a follow-up for the portal repo.

## 7. Per-tool launch behavior

For each tool the launcher does three things: detect, configure, launch.
- **Detect:** macOS: app bundle in `/Applications` or `~/Applications`; `claude` on PATH. Windows: registry uninstall keys or known install paths; `claude` on PATH.
- **Configure:** write the tool's config from the template to the path the tool reads, in the user's own config location (the launcher runs as the user; it does not write to protected paths). For Claude Code, set `apiKeyHelper` to the launcher's own helper command that reads the keychain, so the key never lands in a file. For tools that can only read a key from a file or env var, inject it only into the process the launcher starts; do not write it to disk.
- **Launch:** open the app via the OS (`open -a` / `ShellExecute`), or start the terminal tool in the user's default terminal.
Verify each tool's current config format against its documentation before writing its template. Comment the doc URL and date checked at the top of each template file. Do not write formats from memory. (ChatGPT desktop's custom endpoint configuration in particular must be confirmed; if it does not support one, tell me and leave the tool in the list as "not available yet.")

## 8. Non-goals for v1
- No local proxy. No TLS interception ever.
- No enforcement, watching, or config repair.
- No SSO/OAuth handshake in the launcher.
- No tray icon, autostart, or background process.
- No Pi.
- No logo upload or per-user branding (branding comes from the admin's Deployment config for Claude Desktop, not from the launcher).
- No SQLite.
- No network calls to any host other than the two Consus hosts. No analytics, telemetry, or crash reporting.

## 9. Signing and release
Apple Developer Program enrollment for Consus Industries, Inc. is submitted (D-U-N-S on file) and pending Apple's authority check. Until it clears, builds are unsigned. On Eric's Mac, Gatekeeper will warn on first open; right-click, Open is the expected workaround during development and is not a bug. Set up the CI pipeline so that signing and notarization run when secrets are present and are skipped when absent. Windows: Azure Trusted Signing, not yet set up.

Release artifacts: `.dmg` and `.pkg` (Mac), `.msi` (Windows), published to GitHub Releases with the updater manifest. Homebrew cask and winget manifests are a later step.

## 10. Build order
Each step is its own PR and must run on Eric's Mac before the next starts. **v1 is macOS.** Windows must compile in CI but nothing in this order waits on Windows testing. Eric's Mac has Claude Desktop, Claude Code, and VS Code installed for verification.
1. Scaffold: `cargo create-tauri-app` with the vanilla TypeScript template, bundle id `io.consus.launcher`, app name Consus Launcher. Copy the wireframe's HTML, CSS, and script into `src/` as the UI. First commit opens the window in the cream/ink design with the wireframe rendering and its demo behavior intact.
2. Connect flow: keychain read/write, key validation against `GET /v1/models`, first-run and connected states wired to real data. Remove the demo simulation for this part. This is the only network call in v1.
3. Claude Desktop only: detect, write config from the built-in template with the models from step 2, launch. Get-it state with vendor link when not installed.
4. Claude Code, VS Code: same three behaviors each. ChatGPT tile shows "Coming soon" (v1.1).
5. Keys tab: replace, advanced per-tool, sign out.
6. Log tab: "Coming soon" state until the requests endpoint exists. Keep the wireframe's log UI in place behind it.
7. CI: build.yml for Mac and Windows on every PR (unsigned), release.yml on tag with signing and notarization when secrets exist, skipped when they do not. Updater manifest published with each release.
8. README: what it is, what it does not do (the non-goals, in plain words), what it connects to (the two hosts), how to install, how to build, and where the SBOM is.

## 11. Working style
Show me a plan before building. Follow the build order above, connect flow with keychain, tool detection and launch for Claude Desktop only, one PR at a time. Ask before making a decision that changes scope. If something is not in this document, the default answer is no. Short commit messages. No em dashes anywhere in copy or comments.
