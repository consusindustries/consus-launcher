# Changelog

What changed in each release. Each release's GitHub page shows its section.

## v1.2.0 (unreleased)

- Claude Desktop no longer polls Anthropic's model catalog, checks fetched hostnames with Anthropic in Code sessions, or lets users add plugin marketplaces and uploads. Its remaining Anthropic connection, the sandbox VM bundle downloaded at the first Cowork or Code session, is now stated in the README.
- Claude Code also gets the documented telemetry, error-report, and feedback switches, and the sign-in a desktop launcher can offer is off.
- Web access is a policy. Web search is never offered in Claude Code or Claude Desktop (the gateway refuses it). Fetching web pages from the machine is off by default; the new `WebAccess` org setting turns it on.
- README: which settings the launcher cannot lock (and which managed files from the portal do), CUI means `fedramp-high`, and Codex's `requirements.toml` lock.

## v1.1.0 (2026-10-18)

- Claude Code and Claude Desktop no longer contact Anthropic on their own; only the gateway's calls leave the machine. The launcher turns off their analytics, crash and error reports, feature flags, update checks, Claude Code's plugin marketplace download and WebFetch domain check, and Claude Desktop's connector icons, connector directory, and artifact preview.
- In Claude Desktop, artifacts no longer render, and the app no longer updates itself: install new versions yourself. Claude Code doesn't update itself either; run `claude update`.
- Signing in to a Claude.ai account is turned off, since it would bypass the gateway: `/login` in Claude Code, and the Claude.ai option on Claude Desktop's start screen.

## v1.0.0 (2026-10-03)

- Windows is no longer a preview: the `.msi` and the app are signed by Consus Industries, Inc. (Microsoft Artifact Signing), so Windows shows the publisher instead of "Unknown publisher".
- Says when a newer launcher is out: it reads a public file on `portal.consus.io` and links to the release. It never updates itself. Off by default on machines IT manages; the `UpdateNotice` setting turns it on or off.
- README: how to update and uninstall, and the signing identity IT needs to pre-approve the macOS permissions.
- Tauri 2.12.

## v0.2.4 (2026-10-02)

- ChatGPT gets the Consus key from the launcher's key helper, the way Claude does. On Windows, ChatGPT 26.924 and later opens again: the launcher starts it through its Store package, which the new version requires.
- On macOS, ChatGPT also works when opened from the Dock, once the launcher has set it up.

## v0.2.3 (2026-09-29)

- macOS releases are signed by Consus Industries, Inc. (Apple Developer ID, team K4P2D65BQD) and notarized by Apple, so the `.dmg` and `.pkg` open without a warning.
- The release build retries Apple's notarization upload if it fails.

## v0.2.2 (2026-09-26)

- After connecting, the tool list stays hidden until the launcher has checked which tools are allowed and installed, instead of showing all five for a moment.
- The Windows installer uses Consus artwork.

## v0.2.1 (2026-09-26)

First public release.

- Windows (x64 and ARM64): all five tools, org settings from the registry, and `.msi` installers.
- Org settings from device management on macOS: `EndpointURL`, `ComplianceLevel`, `Tools`, `OrgName`.
- Leaves a tool alone, and labels it "Managed by your organization", when a machine-wide policy already sets how Claude Code or Claude Desktop connects.
- Removes its settings from a tool the organization turns off.
- Model names, limits, reasoning levels, and pricing come from the gateway's `/v1/models` for Pi, Codex, ChatGPT, and Claude Code.

## v0.2.0 (2026-09-25)

Not published; replaced by v0.2.1.

## v0.1.0 (2026-09-24)

Not published. The first build: connect with a Consus key kept in the macOS Keychain, then open Claude Desktop, ChatGPT, Claude Code, Codex, or Pi already set up for Consus.
