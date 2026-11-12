# Changelog

What changed in each release. Each release's GitHub page shows its section.

## v1.5.0 (2026-11-12)

- Factory: Factory's `droid` agent is a sixth tool, on macOS, for organizations that turn it on (it is not in the default set: `factory` in `Tools`, or on in the Consus portal). The launcher opens `droid` in Terminal with its own home and an organization settings file that removes web search and image generation, refuses Factory's own models, and admits models only at your endpoint; prompts and responses go only to Consus. If a `droid` version stops reading that file, the tile refuses to open. Factory still receives your sign-in, its feature flags, and token counts for each turn, and its Automations and `/bug` upload content with no setting to stop them (README, "Tools"). The first launch asks you to sign in to Factory.
- With no `Tools` set, every tool shows but Factory; the Keys tab says "All but Factory".

## v1.4.0 (2026-11-11)

- Connectors: when your organization's admin sets a Slack app's client ID in the Consus portal, the launcher sets Slack up in Claude Desktop and Claude Code, so each person clicks Connect once in each. Slack's write tools are blocked in both apps; the Slack app's read-only scopes are the control. Cleared in the portal, Slack is gone at each app's next launch; sign out removes it. On a machine with a Claude lock file, the lock carries Slack and the launcher adds none (README, "Tools" and "Troubleshooting").
- The Keys tab lists the connectors your organization turned on.

## v1.3.1 (2026-11-11)

- Codex CLI starts again with the launcher's settings. The newest ChatGPT's built-in Codex leaves out a field in its model list that Codex CLI 0.147 requires, so a model list the launcher built from it stopped `codex` with "missing field `supports_parallel_tool_calls`". The launcher now fills the field in. ChatGPT was not affected.

## v1.3.0 (2026-11-11)

- The launcher reads your organization's settings from the Consus portal. An admin chooses the tools, compliance level, web access, and organization name on the portal's Launcher page, and every signed-in launcher picks them up when it starts or its window comes forward, with no file to push. Device management still wins wherever it sets a value. The launcher keeps the last settings it read, so it works offline, and removes them when you sign out.
- If you pushed the Consus Launcher settings profile, remove it to get settings from the portal.
- If your organization's endpoint is a logging proxy, it must forward `GET /v1/me/device-policy` and pass the `If-None-Match` and `ETag` headers through (README, "For IT admins").
- A setting this launcher version does not know, such as a newer compliance level or tool, hides the tools and asks you to update the launcher, instead of falling back to a default.
- A tool your key has no models for shows greyed out with the reason, instead of failing when you open it.
- The Keys tab lists each setting and where it came from: your IT, the Consus portal, this computer, or the default.

## v1.2.4 (2026-11-05)

- The app the Windows installer installs now passes signature verification. Since v1.0.0 the bundler rewrote the signed app to stamp the bundle type into it before packaging, so the installed `consus-launcher.exe` reported a hash mismatch while the `.msi` itself verified. Every release build now installs the x64 `.msi` on the build machine and checks the installed app's signature.
- The Windows installers are named `-windows-x64.msi` and `-windows-arm64.msi`, so the x64 one that most PCs need is listed first on the release page, and the release notes and README say which to pick. The ARM64 installer was listed first, and on an Intel PC it stops with "not supported by this processor type".

## v1.2.3 (2026-11-04)

- The ChatGPT tile shows RUNNING while ChatGPT is open. ChatGPT 26.930 restarts itself in a new process a few seconds after it opens; the launcher took that for the app closing, so the tile went back to OPEN, quitting the launcher left ChatGPT open, and clicking the tile again restarted it. The launcher now follows ChatGPT to the new process.

## v1.2.2 (2026-11-02)

- ChatGPT lists the Consus models again. ChatGPT 26.930 moved a program the launcher uses to build that list, and dropped a model it was built from, so the picker showed OpenAI's models and the gateway refused them.
- If the launcher cannot update ChatGPT's model list, it now says so on the ChatGPT tile. Notices sent while a tool opens stay on the tile instead of being cleared.
- Dependency updates: rustls, sha2, getrandom, Vite, the Tauri CLI, and the rust-cache build action.

## v1.2.1 (2026-10-27)

- While you sign in, the launcher says when your approval has landed and the key is being prepared, instead of still asking you to approve.
- One policy document. Every setting the launcher writes into a tool, and every setting only a device-management lock may carry, now lives in `policy/launcher-policy.json`; the launcher embeds it, the README's per-tool table is generated from it, and each release attaches it so the Consus portal renders the lock files from the same values. What the launcher writes is unchanged.

## v1.2.0 (2026-10-26)

- Sign in with your company account instead of pasting a key. The launcher opens the Consus portal, you approve in the browser, and a key that is yours alone, named after this computer and expiring after 30 days, goes straight to the keychain. Pasting a key stays for organizations without SSO. A key the Consus credential helper already obtained on the machine is used as is.
- Sign out revokes the key in Consus, not just on this computer. The Keys tab shows who is signed in and when the key expires.

- Claude Desktop no longer polls Anthropic's model catalog, checks fetched hostnames with Anthropic in Code sessions, or lets users add plugin marketplaces and uploads. Its remaining Anthropic connection, the sandbox VM bundle downloaded at the first Cowork or Code session, is now stated in the README.
- Claude Code also gets the documented telemetry, error-report, and feedback switches, and the sign-in a desktop launcher can offer is off.
- Web access is a policy. Web search is never offered in Claude Code or Claude Desktop (the gateway refuses it). What the agent reaches from the machine (Claude Code's WebFetch and, on macOS and Linux, its shell through Claude Code's own sandbox; Claude Desktop's Web Fetch and sandbox; the sandbox network of ChatGPT and Codex) is off by default; the new `WebAccess` org setting turns it on. Pi has no sandbox and Windows has none for Claude Code, so their shells are not restricted; the README says so, tool by tool.
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
