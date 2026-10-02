# Changelog

What changed in each release. Each release's GitHub page shows its section.

## Unreleased

- Says when a newer launcher is out: it reads a public file on `portal.consus.io` and links to the release. It never updates itself. Off by default on machines IT manages; the `UpdateNotice` setting turns it on or off.

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
