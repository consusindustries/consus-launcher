# Consus Launcher

Paste your Consus API key once. Click a tool, and it opens already configured for the Consus gateway.

Consus Launcher is a small open source desktop app for macOS. It writes each tool's configuration and launches it. It is a front door, not a gateway and not an enforcement agent.

## Tools

| Tool | How it opens | Where the launcher writes its settings |
|---|---|---|
| Claude Desktop | The app, in gateway mode | `~/Library/Application Support/Claude-3p/configLibrary/` |
| ChatGPT | The app, in Codex mode | `~/.codex/config.toml`, merged key by key |
| Claude Code | Terminal | `~/.claude-consus-gateway/` (its own profile) |
| Codex CLI | Terminal | `~/.codex-consus-gateway/` (its own profile) |
| Pi | Terminal | `~/.pi-consus-gateway/` (its own profile) |

- **Your own setup is left alone.** Claude Code, Codex CLI, and Pi each get a separate profile folder, so `~/.claude` and `~/.pi` are never touched and Codex CLI runs apart from `~/.codex`. ChatGPT has no such option: its settings are merged into `~/.codex/config.toml`, which your own `codex` also reads. Terminal tools start in an empty `~/Consus` folder.
- **Models:** each tool is set up with the ITAR models your key can use, from `GET /v1/models` (ChatGPT starts on one; the others list them all).
- **The key** lives in the macOS Keychain and is never written to a file. Tools fetch it through the launcher itself (a keychain helper), or receive it in the environment of the process the launcher starts.
- **Windows:** a tool opens inside the launcher's glass area, then moves freely. Quitting the launcher quits the tools it started, never ones you opened yourself.
- **Claude and ChatGPT restart once.** If either app is already open when you click it, the launcher asks it to quit and reopens it in gateway mode, because both read their settings only at startup.
- **Sign out** removes the launcher's settings from each tool and keeps everything else, including your history.
- A tool that is not installed links to its vendor. Consus never installs software.

## What it does not do

- No proxy and no TLS interception, ever.
- No enforcement, monitoring, or config repair.
- No sign-in flow; you paste a key.
- No tray icon, no autostart, no background process.
- No local database; no analytics, telemetry, or crash reporting.
- No auto-update. New versions come from the [Releases](https://github.com/consusindustries/consus-launcher/releases) page.

## What it connects to

The launcher makes one kind of network request: `GET https://api.consus.io/v1/models` (or the same path on your organization's endpoint, if your IT admin set one), when you connect and each time the launcher starts, to check the key and list your models. `https://portal.consus.io` only ever opens in your browser. (The Terminal installer downloads from GitHub; the app itself never does.)

The tools you open send their requests to `api.consus.io`, or to your organization's endpoint, with the settings the launcher writes. What else a tool does is up to the tool. For ChatGPT and Codex, the launcher's config turns off analytics, feedback, and OpenTelemetry export, and Codex's update check; for Pi, install telemetry and the update check. On its first run, Pi downloads `fd` and `ripgrep` from GitHub, because its search tools need them.

## For IT admins: org settings

The launcher reads settings for the `io.consus.launcher` preferences domain. Push them in a configuration profile with your device management (Jamf, Intune, and so on); a value from a profile wins over the user's own preferences. Every key is optional.

| Key | Type | What it does |
|---|---|---|
| `EndpointURL` | string | Where every tool sends its requests, for example your own logging proxy. Default `https://api.consus.io`. Must be `https://` (plain `http://` only to `localhost`). |
| `ComplianceLevel` | string | The compliance level every tool is set up for: `itar` (default), `fedramp-high`, `fedramp-high+itar`, and the other levels the gateway supports. |
| `Tools` | array of strings | The tools shown: `claude-desktop`, `chatgpt-desktop`, `claude-code`, `codex-cli`, `pi`. Default: all. |
| `OrgName` | string | Shown in the launcher's header. |

The launcher reads these when it starts. If a value is present but invalid, the launcher explains the problem and sets nothing up, rather than falling back to defaults. Hiding a tool here is a convenience, not a control: what a key can do is enforced by the Consus gateway.

To try settings on one Mac without a profile:

```sh
defaults write io.consus.launcher Tools -array claude-code codex-cli
defaults write io.consus.launcher OrgName "Example Org"
defaults delete io.consus.launcher    # back to defaults
```

## Install

Requires macOS on Apple silicon or Intel (tested on macOS 26), and a Consus API key from your Consus admin.

**From Terminal** (recommended until releases are notarized by Apple):

```sh
curl -fsSL https://raw.githubusercontent.com/consusindustries/consus-launcher/main/install.sh | sh
```

[`install.sh`](install.sh) downloads the latest release from GitHub, checks the disk image against the release's `SHA256SUMS.txt`, installs Consus Launcher into `/Applications` (or `~/Applications`), and opens it. Nothing is installed unless the checksum matches. Set `CONSUS_LAUNCHER_VERSION` to pick a release (for example `v0.1.0`); to install from an internal mirror of the release files, set `CONSUS_LAUNCHER_BASE_URL` and `CONSUS_LAUNCHER_VERSION` together.

The checksum catches a corrupted or mismatched download; it does not by itself prove who built the release. This install avoids the Gatekeeper prompt because files fetched with `curl` are not marked as downloaded from the internet, so macOS does not check them on first open. Until releases are notarized, that trade-off is the reason to prefer it or not.

**From the download:**

1. Download the `.dmg` (drag to Applications) or the `.pkg` (installs to `/Applications`) from [Releases](https://github.com/consusindustries/consus-launcher/releases).
2. Optionally, check the download against `SHA256SUMS.txt` with `shasum -a 256 -c --ignore-missing SHA256SUMS.txt`.
3. Open Consus Launcher and paste your key.

**Until releases are notarized,** macOS blocks a downloaded copy on first open. Go to System Settings, Privacy and Security, and click Open Anyway. On some Macs the approval never appears and the app stays stuck opening; use the Terminal install instead.

On first use, macOS asks for:

- **Keychain access**, to read your key. Choose Always Allow.
- **Automation**, to open terminal tools in Terminal and to place windows.
- **Accessibility** (System Settings, Privacy and Security, Accessibility), to place app windows inside the launcher. Without it, apps still open, just not in place.

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

- `Consus-Launcher-<version>-universal.dmg` and `.pkg`
- `consus-launcher-rust.cdx.json` and `consus-launcher-npm.cdx.json`: CycloneDX SBOMs for the Rust crates and the npm packages in the app
- `SHA256SUMS.txt`

Signing and notarization run in that same workflow once the Apple secrets are set; the secret names are listed at the top of `release.yml`. Run the workflow by hand (Actions, Release, Run workflow) to try signing before tagging.

To cut a release:

1. Set the new version with `npm version --no-git-tag-version <version>` (updates `package.json` and `package-lock.json`), then in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`. Commit it with the updated `Cargo.lock`. The workflow refuses to build if any of these differ.
2. Tag the commit `v<version>` and push the tag.
3. The workflow drafts a GitHub Release with the files above. Review it, then publish.

## License

[Apache-2.0](LICENSE)
