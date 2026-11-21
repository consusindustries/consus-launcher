# VS Code: a seventh tool

*How the launcher opens VS Code against the Consus gateway (launcher 1.7.0). Drafted
2026-11-19 from runs on VS Code 1.132.0 and 1.147.0 (macOS and Windows 11 ARM64,
2026-11-18) and from the setting and model schemas inside VS Code 1.147.0's own bundle.
Counterpart: the Consus portal's Launcher page, which turns the tool on. The gateway
needs no change.*

## What changes, in three sentences

VS Code becomes the seventh tile, opt-in, on macOS and Windows: an admin turns it on in
the portal, and the launcher then opens VS Code's own app as a separate instance, with
its own `--user-data-dir` and `--extensions-dir` under `~/.vscode-consus-gateway`,
set up with the Consus models the key has, and IT can push VS Code's managed policies
from the portal as the lock. VS Code has no way to take a key from another
program, so each time it opens the launcher shows the steps (trust the folder, then four) and a "Copy key" button,
and the person pastes the key into VS Code once (again after a new key). The person's
own VS Code folders are never read or written, and the launcher never closes, moves, or
signals a VS Code it did not start.

## What was seen (2026-11-18)

- **VS Code's chat and its Custom Endpoint provider ship inside VS Code**
  (`app/extensions/copilot`). Nothing is downloaded or installed.
- **A brand-new instance hides chat behind a GitHub sign-in**: VS Code disables its
  built-in chat extension the first time an instance starts, unless setup already ran.
  It records that it ran under the state key `builtinChatExtensionEnablementMigration`.
  With that row already in `User/globalStorage/state.vscdb` before the first start,
  there is no sign-in screen and the Consus model is listed and selected (1.132.0 and
  1.147.0, macOS and Windows).
- **VS Code does not ask for the key.** With no stored key it sends an empty key, and
  the gateway answers 403. The key is set through VS Code's own menu, and the model
  then answers (1.147.0, macOS and Windows):
  1. Click the model name in the chat box, then "Manage Models...".
  2. Right-click the model row (or the gear at its end) and pick "Update API Key".
  3. Paste, press Enter.
  4. Start a new chat.
- **After "Update API Key"**, VS Code stores the key in its secret storage and rewrites
  the group's `apiKey` to `${input:chat.lm.secret.<id>}`, keeping the group's name and
  models. So the launcher keeps whatever `apiKey` the group has on every later write.
- **A folder window opens in Restricted Mode** and lists no models until the folder is
  trusted. The launcher opens a new instance on `~/Consus`, as every tool starts there,
  and leaves VS Code's trust prompt on: trusting the folder is the first step shown.
  VS Code remembers it.
- **A chat with no Consus model selected** (for example "Auto") asks for a GitHub
  sign-in. So the launcher always sets the default model.
- **VS Code shares some state across all its instances**:
  `~/.vscode-shared/sharedStorage/state.vscdb` holds the recently opened list and the
  folder trust list. No secrets. A folder trusted in one instance is trusted in the other.
  The launcher never writes that file.
- **The binary refuses its flags** ("bad option: --user-data-dir") when
  `ELECTRON_RUN_AS_NODE` is set, so the launcher removes it from VS Code's environment.
- **`code chat` and the `code` command's other subcommands ignore `--user-data-dir`** and
  act on the person's own running VS Code. The launcher starts the app binary only.
- **A short host check** (1.147.0, macOS, signed out, telemetry and updates off, one
  chat message): `main.vscode-cdn.net` and `marketplace.visualstudio.com` while idle, and
  the Consus endpoint for the chat. No GitHub or Copilot host. This is not the release
  measurement (check 8).

## What the launcher writes

All under `~/.vscode-consus-gateway/data/User/` (`%USERPROFILE%` on Windows):

- **`chatLanguageModels.json`**, an array of provider groups. The launcher owns the one
  named `Consus Gateway` (vendor `customendpoint`, `apiType: chat-completions`) and leaves
  any other group alone. Its `apiKey` is `${input:consusApiKey}` on first write, and after
  that whatever VS Code put there. Its models are every chat model the key has at the
  org's level, Claude first, then by maker and name, in the org's model order when it has
  one. Each model has: `id` (the bare id with its level), `name` (display name and level
  tag), `url` (the endpoint's `/v1`), `requestHeaders: {"x-api-key": "${apiKey}"}`,
  `toolCalling: true`, `vision`, `maxInputTokens`, and `maxOutputTokens`. Models the
  gateway reports reasoning efforts for also get `thinking: true`,
  `supportsReasoningEffort` (the efforts) and `reasoningEffortFormat: chat-completions`.
- **`settings.json`**, merged at the top level only. The launcher owns the names in the
  policy document's `tools.vscode.settings`, both `by_web_access` variants, and three it
  computes:
  - `chat.defaultModel`: the org's start model, else the newest Opus, else the first
    model, as `customendpoint/Consus Gateway/<id>`. That is the model's id in VS Code
    1.147's default (Copilot) session type, seen on Windows 2026-11-19. A Local chat
    does not match it and starts on the first Consus model.
  - `chat.utilityModel` and `chat.utilitySmallModel`: the first Sonnet and the first
    Haiku, else the default model, as `customendpoint/<id>`. That is the form VS Code
    1.147 stores for these settings.

  A file VS Code cannot parse as plain JSON (VS Code allows comments) is an error, and
  neither file is written.
- **`globalStorage/state.vscdb`**, only if it does not exist yet: the one-row database
  above, shipped inside the launcher (`src-tauri/assets/vscode-state.vscdb`). It is made
  with:

  ```
  sqlite3 src-tauri/assets/vscode-state.vscdb \
    "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
     INSERT INTO ItemTable VALUES ('builtinChatExtensionEnablementMigration','true');"
  ```

  It is written to a temporary name and hard-linked into place, so an existing file is
  never replaced and VS Code never sees a partial one.

Sign out, or the org turning VS Code off, removes the group and the owned settings
names. The folder, its chat history and the state file stay. The key VS Code stored
stays in VS Code's own store; sign out revokes it, so it no longer works.

## Starting, tracking, closing

- **Found by its install, never by name.**
  - macOS: `Visual Studio Code.app` in `/Applications` or `~/Applications`.
  - Windows: `%LOCALAPPDATA%\Programs\Microsoft VS Code\Code.exe`, then
    `%ProgramFiles%\Microsoft VS Code\Code.exe`, then the install folder from VS Code's
    uninstall entry (HKCU, HKLM, and HKLM `WOW6432Node`). Never `bin\code.cmd`.
    Its app folder (version, icon) is `resources\app` beside `Code.exe`, else under a
    versioned folder (`<install>\8a14f0cc1b\resources\app` on 1.147.0).
- **The version** comes from the install's `app/package.json`. Below the minimum (1.147.0
  until check 11 says otherwise), the tile is greyed with "Update VS Code to … or later".
- **Started as** `<binary> --user-data-dir <folder>/data --extensions-dir
  <folder>/extensions`, plus `~/Consus` when the instance is new (its state file was
  just written). Later starts pass no folder, so VS Code reopens what was open.
  The environment drops `CLAUDE*`,
  `ANTHROPIC*`, `OPENAI*`, `CODEX*`, `VSCODE_*`, `ELECTRON_RUN_AS_NODE` and `CONSUS_API_KEY`.
- **Tracked by the pid the launcher started.** VS Code keeps one main process per
  user-data-dir, so that pid is the launcher's even while the person's own VS Code runs.
- **A second click while it runs** starts the binary again with the same folders. That
  process hands over to the running one and exits, and the files are not rewritten.
  `window.openWithoutArgumentsInNewWindow: off` makes VS Code focus its window instead of
  opening a new one.
- **Closed at sign out and at launcher exit**, by that pid only: `kill -TERM`, then
  `-KILL` after fifteen seconds (macOS); `taskkill /PID` (its window is asked to close, and VS Code
  ends its own helpers), then `taskkill /PID /T /F` after fifteen seconds (Windows). Never
  AppleScript, a bundle id, or a process name.
- **Not placed in the glass.** The launcher does not go behind it, so the steps panel
  stays reachable.

## The key, by the clipboard

The steps panel shows every time VS Code opens (it is needed only the first time and
after a new key, but showing it always means the person always knows what to do). Its
"Copy key" button puts the key on the clipboard:
- **From Rust only.** It is read from the keychain, and the webview gets no clipboard
  permission.
- **Marked as a secret.** On macOS it carries `org.nspasteboard.ConcealedType`. On
  Windows it carries `CanIncludeInClipboardHistory = 0`, `CanUploadToCloudClipboard = 0`
  and `ExcludeClipboardContentFromMonitorProcessing`.
- **Cleared** after two minutes, or when the launcher quits, if nothing else was copied
  since. The launcher compares the system's change counter and never reads the clipboard.

The panel also says not to sign in to GitHub in this VS Code: signing in would add
GitHub Copilot's own models next to Consus.

## The policy document

`tools.vscode` has `settings`, `by_web_access` and the six `readme` cells, and
`lock: null`. Its keys are literal VS Code setting names, unlike the other tools'
nested keys. The values are provisional until the checks below.

- `settings`:
  - `telemetry.telemetryLevel: off`, `telemetry.feedback.enabled: false`,
    `update.mode: none`, `extensions.autoUpdate: false`,
    `extensions.autoCheckUpdates: false`, `workbench.startupEditor: none`,
    `window.openWithoutArgumentsInNewWindow: off`, `chat.sessionSync.enabled: false`,
    `chat.agent.sandbox.allowUnsandboxedCommands: false`.
  - `chat.byokUtilityModelDefault: mainAgent`: titles and summaries use the chat's own
    model, not Copilot's, when no utility model resolves.
  - `chat.titleBar.signIn.enabled: false`, `chat.welcomePage.signIn.enabled: false`: no
    sign-in nudges.
  - `workbench.welcomePage.experimentalOnboarding: false`: VS Code 1.147 shows a
    first-start onboarding dialog ("Welcome to VS Code, sign in to use GitHub Copilot")
    in every new instance while this is on, Consus models or not (seen on Windows,
    2026-11-19).
  - The session type is left at VS Code's default (Copilot). Its agent sandbox is the one
    that also runs on Windows; the local one is macOS and Linux only. With
    `chat.defaultModel` set, new chats start on a Consus model and need no GitHub sign-in
    (check 5).
- `by_web_access.off`: `chat.agent.networkFilter: true`,
  `chat.agent.sandbox.network.allowedDomains: []`, `chat.agent.sandbox.enabled: on`,
  `chat.agent.sandbox.network.allowNetwork: false`. The filter covers the fetch tool and
  the built-in browser and allows only the listed domains (none); the sandbox covers
  commands the agent runs. Check 7.
- `by_web_access.on`: `chat.agent.networkFilter: false`, `chat.agent.sandbox.enabled: off`.

## The lock

VS Code reads managed policies from the `com.microsoft.VSCode` preferences domain on
macOS and `HKLM\SOFTWARE\Policies\Microsoft\VSCode` on Windows. The portal renders them:
- **What goes in:** every key in `settings` and the active `by_web_access` variant that
  `policy_names` maps, under its policy name: `TelemetryLevel`, `ChatAgentNetworkFilter`,
  `ChatAgentAllowedNetworkDomains`, `ChatAgentSandboxEnabled`,
  `ChatAgentSandboxAllowNetwork`. Also `lock_only.required`:
  `ChatAgentSandboxAllowUnsandboxedCommands`, `CopilotSessionSync` and `EnableFeedback`,
  all false.
- **Value types:** on macOS, the profile uses native types, as in VS Code's own sample
  (`Contents/Resources/app/policies/com.microsoft.VSCode.mobileconfig`).
  `ChatAgentAllowedNetworkDomains` is a string holding a JSON array, and
  `ChatAgentSandboxEnabled` the string `on` or `off`.
- **Left out on purpose:** `UpdateMode`, `ExtensionsAutoUpdate` and `AllowedExtensions`.
  The lock is machine-wide, so it applies to the person's own VS Code too, and these would
  stop its updates and restrict its extensions.
- **No policy blocks a GitHub sign-in**, so the "do not sign in to GitHub" line stays
  under the lock.
- **Under the lock the launcher still writes its files**: the model list, the state file
  and its settings. Policies cannot carry them. The tile reads "Managed by your
  organization". The launcher sees the lock when `ChatAgentNetworkFilter` is set in a
  configuration profile for that domain (`/Library/Managed Preferences`, machine or user)
  or under the registry key.

## Checks still to run

A person has to be at the screen for these. Each result is recorded here as "Done,
date, VS Code version, OS".

| # | Check | Status |
|---|---|---|
| 3 | What VS Code shows with a revoked or wrong stored key; where 1.147 keeps `chat.lm.secret.*` on disk (README wording) | Open |
| 4 | A second start with the same folders focuses the same window; with the person's own VS Code open, ours starts separately and closing ours leaves theirs open; with ours running, does a Dock or Spotlight click on VS Code bring up ours (macOS)? | Windows done 2026-11-19, 1.147.0, Windows 11 ARM64: the second start opens no window and exits after about 16 s; a second instance in another folder is untouched; `taskkill /PID` closes ours in 2 to 9 s with its helpers, while `taskkill /PID /T` closes nothing (its helpers refuse anything but `/F`). macOS open |
| 5 | The sign-in nudges are gone; new chats never start on "Auto"; chat and agent mode still work | Partly, Windows 2026-11-19, 1.147.0: a new instance showed the first-start onboarding dialog until `workbench.welcomePage.experimentalOnboarding: false` was added; with it, no sign-in dialog. Auto and agent mode open |
| 6 | `chat.defaultModel` selects the Consus model in a new chat; titles and summaries go to Consus | Partly, Windows 2026-11-19, 1.147.0: the launcher's model file loads ("Wired 4 BYOK model(s)"), and in the Copilot session type the models' ids are `customendpoint/Consus Gateway/<id>`, so `chat.defaultModel` now uses that form (a bare id did not match, and the chat started on the first model). Confirm on screen; utility models open |
| 7 | With web access off, the fetch tool and browser are refused and a `curl` the agent runs is blocked by the sandbox, in both session types, macOS and Windows; with it on, both work | Open |
| 8 | Every request through a logging proxy with a canary, on the built launcher, agent mode with tools (method as in `docs/factory.md`); fills the `readme` cells | Open |
| 9 | A Grok model answers with no `modelOptions` | Open |
| 10 | Windows: the all-users install path, and check 4 | Check 4 done (above). The per-user install, its versioned app folder, the uninstall entry, and the launcher's clipboard code (text, the three history and monitor flags, the clear, a later copy surviving the clear) checked 2026-11-19. All-users install open |
| 11 | The oldest VS Code where the key steps work; sets the minimum | Open (1.147.0 until then) |
| 12 | The lock on macOS and Windows: each value shows as managed and cannot be changed; registry types match the ADMX files; the launcher's VS Code still lists the Consus models and answers; what it does to the person's own VS Code | Open |

## Out of scope

- An extension that would remove the paste.
- The Claude Code and Codex extensions inside VS Code, tab completions, semantic search,
  embeddings, and MCP servers in VS Code.
- Linux.
