# Consus Launcher: project notes

First written 2026-09-21 as the build handoff; rewritten 2026-09-24 to describe the app as built. The original wireframe (`docs/consus-launcher.html`) and screenshots (`docs/screenshots/`) predate later changes such as the Log tab removal.

Guiding rule: this is the simplest application possible. It stores one key, writes each tool's config, and launches the tool. Anything beyond that is out of scope unless Eric says otherwise.

## 1. What it is

A small open source desktop app. The user pastes a Consus API key once. After that, clicking a tool opens it already configured for the Consus gateway.

It is a front door, not a gateway and not an enforcement agent. It writes configs and launches apps. Nothing else.

How it fits with the rest of Consus: the **gateway** (`api.consus.io`) enforces policy on every request; the **portal** (`portal.consus.io`) is where people get keys and admins set policy; the **launcher** sets up each person's tools.

## 2. Decisions

Original decisions (2026-09-21), still in force:
- **App, not a daemon.** No tray process, no background service, nothing resident.
- **Consus enforces nothing on the machine.** The launcher does not watch, reset, or block configs. Enforcement is the gateway's job.
- **Consus installs nothing third-party.** A tool that is not installed links to its vendor.
- **Paste-a-key onboarding.** The user creates a key at portal.consus.io and pastes it. The launcher validates it with `GET /v1/models`.
- **Key lives in the OS keychain.** Never written to a file by the launcher. Never shown after entry; only the last four characters.
- **Open source**, Apache-2.0, public repo. Releases on GitHub; signed once the Apple enrollment clears (section 9).

Decided since (Eric's calls):
- **Tools** (2026-09-24): Claude Desktop, ChatGPT, Claude Code, Codex CLI, Pi. VS Code is dropped.
- **Your own setup is left alone** (2026-09-23): terminal tools get their own profile folders (`~/.claude-consus-gateway`, `~/.codex-consus-gateway`, `~/.pi-consus-gateway`) and start in an empty `~/Consus` folder. The launcher never reads or writes `~/.claude` or `~/.pi`. ChatGPT has no such option: its settings are merged into `~/.codex/config.toml`, which the user's own `codex` also reads, so that `codex` routes to Consus until sign out.
- **Standalone windows** (2026-09-22): a launched app is placed in the launcher's glass area once, then moves freely. Quitting the launcher quits the apps it started, never ones the user opened.
- **Compliance level is ITAR by default** (2026-09-22); an org can set another level through org settings (2026-09-24).
- **Terminal.app only** for terminal tools on macOS; a terminal picker is later.
- **One key.** The per-tool key override and the Log tab placeholder were removed (2026-09-24); org settings and the portal's usage view cover those needs.
- **No auto-update, drafted releases** (2026-09-24). Releases were Mac-only until Windows support landed; they now include a Windows `.msi` (preview, 2026-09-25). See section 9.
- **Line count** (2026-09-24): about 1,500 lines of app code is a guideline, not a limit.

## 3. Repo

Public, Apache-2.0. Layout:
```
index.html            the UI's page (Vite root)
src/                  main.ts, styles.css, assets/ (fonts, Consus logo)
src-tauri/
  src/                main.rs, lib.rs, keychain.rs, portal.rs, models.rs, tools.rs,
                      config.rs (Claude Desktop), chatgpt.rs, claude_code.rs, codex.rs, pi.rs
  templates/          chatgpt-desktop.toml
  assets/             pi.svg
  icons/              app icons
  capabilities/       Tauri permission set
  tauri.conf.json     bundle id io.consus.launcher
  Info.plist, Entitlements.plist
docs/                 these notes, original wireframe, screenshots
.github/workflows/    build.yml (Mac + Windows, every PR), release.yml (tagged releases)
README.md  SECURITY.md  CONTRIBUTING.md  NOTICE  LICENSE
```

### Supply chain and network rules (customers review this)
- **Network:** the launcher talks to `api.consus.io` (`GET /v1/models`), or the org's endpoint when org settings set one, and opens `portal.consus.io` in the browser. No other host. Fonts are bundled.
- **Dependencies:** exact versions pinned, `Cargo.lock` committed, short list a security team can read in a minute. The UI has no runtime packages beyond Tauri's own. Ask before adding one.
- **Actions:** third-party GitHub Actions pinned to commit SHAs.
- **SBOM:** CycloneDX for the Rust crates and the npm packages, attached to every release.
- **Provenance:** release binaries are built only in GitHub Actions, never on a laptop.

## 4. Stack

- **Tauri 2** (Rust core, system webview), frameless transparent window (`macOSPrivateApi`).
- **Frontend:** plain HTML, CSS, TypeScript.
- **Crates:** `tauri`, `tauri-plugin-opener` (opens links in the browser), `serde`, `serde_json`, `keyring` (keychain), `reqwest` (the one API call), `toml_edit` (merging TOML configs), `base64`.
- **No settings file and no updater.** The launcher keeps its key-helper links in `~/Library/Application Support/io.consus.launcher/` and an icon cache in `~/Library/Caches/io.consus.launcher/`; SECURITY.md lists every file it writes.
- **Bundle identifier:** `io.consus.launcher` (Tauri config, keychain service, config directory). Do not change it after the first signed release.

## 5. Screens and behavior

- **Window:** frameless and resizable. Left is transparent glass; right is a 300px panel.
- **First run:** "Connect to Consus": open the portal, create a key, paste it. Connect calls `GET /v1/models`; on success the key goes into the keychain.
- **Open tab:** one tile per tool with its real icon. States: Open, Running, Get it (not installed, links to the vendor).
- **Keys tab:** the key's last four characters, model count, Replace, and "Sign out and remove keys" (deletes the keychain entry and removes the settings the launcher wrote from every tool).
- **Revoked key:** at startup, a 401/403 removes the stored key and returns the user to first run with a plain message. On Connect or Replace, the key is simply not accepted.

## 6. Consus services the launcher depends on

- `GET /v1/models` with `x-api-key`: validates the key and lists the models it can use.

Org settings (built 2026-09-24): read from the `io.consus.launcher` managed preferences a device-management profile sets (keys in the README): allowed tools, compliance level, endpoint, org name. Next (section 10): the same settings from the portal, fetched with the user's key.

## 7. Per-tool behavior

| Tool | Detect | Configure | Key | Launch |
|---|---|---|---|---|
| Claude Desktop | `Claude.app` | Gateway entry in `~/Library/Application Support/Claude-3p/configLibrary/` | Launcher binary as the credential helper | The app, placed in the glass |
| ChatGPT | `ChatGPT.app` | Compliance template merged key by key into `~/.codex/config.toml` | Environment of the process the launcher starts | The app, placed in the glass |
| Claude Code | `claude` on PATH or the login shell | `~/.claude-consus-gateway/settings.json` | `apiKeyHelper` pointing at the launcher | Terminal, in `~/Consus` |
| Codex CLI | `codex`, or the one inside ChatGPT.app | `~/.codex-consus-gateway/config.toml` + model catalog | Fetched from the keychain helper into Codex's environment | Terminal, in `~/Consus` |
| Pi | `pi` | `~/.pi-consus-gateway/models.json` + default model | Pi runs the launcher's helper per request | Terminal, in `~/Consus` |

Each template, or the module that embeds it, records the doc it was checked against and the date. Configs are merged, never overwritten: the launcher owns only its keys, and sign out removes only those.

## 8. Non-goals
- No local proxy. No TLS interception ever.
- No enforcement, watching, or config repair.
- No sign-in flow in the launcher (paste a key).
- No tray icon, autostart, or background process.
- No local database.
- No network calls beyond the Consus hosts. No analytics, telemetry, or crash reporting.

## 9. Signing and release
- `release.yml` builds a universal `.dmg` and `.pkg` on a `v*` tag, attaches SBOMs and checksums, and drafts the GitHub Release for a person to publish.
- **Signing and notarization** run when the Apple secrets exist (names at the top of `release.yml`) and are skipped otherwise. The Apple Developer enrollment for Consus Industries, Inc. is pending.
- **Until notarized,** current macOS blocks a browser-downloaded build. On one test Mac it hung with no approval dialog, while a command-line download installed and ran normally. The first public release waits for notarization; a command-line installer is the interim path.
- **Windows:** `.msi` for x64 and ARM64 in each release (preview). Not code-signed until Azure Trusted Signing is set up; window placement in the glass is not planned on Windows (Eric, 2026-09-25).
- **No in-app updater:** it would be a network host beyond the Consus hosts.

## 10. Status and what's next
Done: connect flow, five tools on macOS, glass placement, quit together, sign out, CI with tests, release pipeline, README, Terminal installer, org settings from device management, removing a turned-off tool's settings, model names and limits from the gateway's `/v1/models` (Pi's list, the Codex and ChatGPT catalog, Claude Code's context window).

Next, in order:
1. Org settings from the portal, fetched with the user's key, once the portal serves them (later; device management covers it until then). The design is kept with the Consus services.
2. Signed, notarized releases once Apple enrollment clears; device-management kit (`.pkg` plus configuration profile).
3. Windows: tool detection, configs, launch, quit-with-launcher, org settings from the registry, and `.msi` releases (preview, 2026-09-25). Code signing waits on Azure Trusted Signing.

## 11. Working style
Plan before building. One PR at a time, reviewed, with CI green before merge. Ask before a decision that changes scope; if something is not in these notes, the default answer is no. Short commit messages. No em dashes anywhere in copy or comments.
