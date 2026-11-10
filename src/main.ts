import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";

const PORTAL_URL = "https://portal.consus.io";
// Tools the launcher detects, configures, and launches.
type Tool = "desktop" | "chatgpt" | "code" | "codex" | "pi";
const TOOLS: Tool[] = ["desktop", "chatgpt", "code", "codex", "pi"];
// One entry per tool: the panel's tiles and the icon's menu are drawn from
// this list. `from` is where the vendor's download lives, as shown.
const TOOL_INFO: Record<Tool, { name: string; sub: string; glyph: string; download: string; from: string }> = {
  desktop: { name: "Claude Desktop", sub: "Chat, docs, analysis", glyph: "C", download: "https://claude.ai/download", from: "claude.ai/download" },
  chatgpt: { name: "ChatGPT", sub: "Chat, docs, analysis", glyph: "G", download: "https://openai.com/chatgpt/download/", from: "openai.com" },
  code: { name: "Claude Code", sub: "Terminal agent", glyph: ">_", download: "https://claude.com/claude-code", from: "claude.com/claude-code" },
  codex: { name: "Codex", sub: "Terminal agent", glyph: ">_", download: "https://developers.openai.com/codex", from: "developers.openai.com/codex" },
  pi: { name: "Pi", sub: "Terminal agent", glyph: "π", download: "https://pi.dev", from: "pi.dev" },
};
const notInstalled = (t: Tool) => "Not installed · get it from " + TOOL_INFO[t].from;
const IS_WINDOWS = navigator.userAgent.includes("Windows");
const isTool = (k: string): k is Tool => (TOOLS as string[]).includes(k);
// Tools the launcher has running right now; more than one can be up.
const running = new Set<Tool>();

interface ConnectResult {
  models: unknown;
  model_count: number;
  email: string | null;
}

type ConnectError =
  | { kind: "Revoked" }
  | { kind: "Network"; message: string }
  | { kind: "Rejected"; message: string };

interface KeyInfo {
  key: string;
  mask: string;
  user: string;
  models: number;
  added: string;
}

// Who signed in, from the launcher's signin.json (no secret in it).
interface SigninMeta {
  owner: string;
  org_id: string;
  key_id: string;
  label: string;
  expires: string;
  source: "signin" | "helper" | "pasted" | "";
}

type SigninError =
  | { kind: "NotStarted" }
  | { kind: "Expired" }
  | { kind: "Refused"; message: string }
  | { kind: "Network"; message: string };

let signinMeta: SigninMeta | null = null;
// The sign-in attempt in progress, if any; a new attempt or Cancel retires
// the old one, whose loop then stops at its next turn.
let signinAttempt = 0;
// How long one attempt waits for the approval, as the helper script does.
const SIGNIN_BUDGET_MS = 10 * 60 * 1000;

interface KeyStore {
  def: KeyInfo | null;
}

let cur: Tool | null = null;
let K: KeyStore = { def: null };

function $<T extends HTMLElement = HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
}

function esc(x: string): string {
  return String(x).replace(/[&<>"]/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c,
  );
}

function mask(k: string): string {
  return "••••" + k.slice(-4);
}

function validate(k: string): string | null {
  k = (k || "").trim();
  if (!k) return "Paste a key first.";
  if (k.length < 16) return "That key looks cut off. Copy it again from the portal.";
  return null;
}

const REVOKED_MESSAGE = "This key was revoked. Sign in again.";

// Portal commands reject with a ConnectError object; keychain commands reject with a string.
function connectErrorMessage(err: unknown): string {
  if (typeof err === "string") return "The portal accepted the key, but it could not be saved to your keychain. (" + err + ")";
  if ((err as ConnectError).kind === "Network") {
    const detail = (err as { message?: string }).message;
    return "Could not reach the portal. Check your connection." + (detail ? " (" + detail + ")" : "");
  }
  if (settingsError) return settingsError;
  return "That key was not accepted by the portal.";
}

let userConnected = false;
let currentModels: unknown = null;
// Set when the org's launcher settings (from device management) are invalid:
// the launcher then shows why instead of configuring anything.
let settingsError: string | null = null;

interface SettingsView {
  org_name: string | null;
  managed: boolean;
  error: string | null;
  web_access: boolean;
}

let webAccess = false;

async function loadSettings(): Promise<void> {
  const s = await invoke<SettingsView>("get_settings");
  $("orgName").textContent = s.org_name ?? "Consus";
  settingsError = s.error ? s.error + (s.managed ? " Contact your IT admin." : "") : null;
  webAccess = s.web_access;
}

function applyConnectResult(rawKey: string, result: ConnectResult): KeyInfo {
  currentModels = result.models;
  const owner = signinMeta?.owner || result.email || null;
  return {
    key: rawKey,
    mask: mask(rawKey),
    user: owner ?? mask(rawKey),
    models: result.model_count,
    added: new Date().toLocaleDateString(),
  };
}

// A freshly minted key takes a few seconds to be accepted at the gateway.
// Ask until it is, for up to half a minute, then report whatever came back.
async function validateWithPatience(key: string): Promise<ConnectResult> {
  const until = Date.now() + 30_000;
  for (;;) {
    try {
      return await invoke<ConnectResult>("validate_key", { key });
    } catch (err) {
      if ((err as ConnectError).kind !== "Revoked" || Date.now() > until) throw err;
      await new Promise((r) => setTimeout(r, 2000));
    }
  }
}

function setConnected(on: boolean): void {
  $("connect").style.display = on ? "none" : "block";
  $("main").style.display = on ? "flex" : "none";
  $<HTMLButtonElement>("org").disabled = !on;
  showAccount(false);
  keyWarn = false;
  renderStatus();
  if (on && K.def) {
    showHint();
    $("orgUser").textContent = K.def.user;
    $("whoNote").textContent = "";
    $("acctNote").textContent =
      "Signed in as " + K.def.user + ". Every tool here goes through Consus. Anything not installed links to the vendor; Consus never installs software.";
    $("webNote").textContent = webAccess
      ? "Web access is on: tools may fetch web pages from this computer. Your organization sets this."
      : "Web access is off: tools can't fetch web pages from this computer. Your organization sets this.";
    renderKeys();
    void refreshTools();
  } else {
    void invoke("set_tray_menu", { tools: [], update: updateVersion });
    $("orgUser").textContent = "";
    ($("keyIn") as HTMLInputElement).value = "";
    $("keyErr").textContent = "";
    (["c1", "c2", "c3"] as const).forEach((c) => ($(c).className = "cn"));
    $("c1").textContent = "1";
    $("c2").textContent = "2";
    $("c3").textContent = "3";
  }
  syncTray();
}

// Signed in, and the last check of the key could not reach Consus.
let keyWarn = false;
// A tool sent a notice, or a newer launcher turned up, since the panel was last open.
let unseen = false;
let updateVersion: string | null = null;

function renderStatus(): void {
  const on = !!K.def;
  const st = $("status");
  st.classList.toggle("off", !on);
  st.classList.toggle("warn", on && keyWarn);
  $("statusT").textContent = !on ? "NOT CONNECTED" : keyWarn ? "CAN'T REACH CONSUS" : "CONNECTED";
  st.title = keyWarn ? "The last check of your key could not reach Consus. Tools may not work until it can." : "";
}

// The icon follows the panel: orange when signed in, grey when not, and a
// dot when something here wants a look.
function syncTray(): void {
  const connected = !!K.def && $("main").style.display !== "none";
  void invoke("set_tray_state", { connected, attention: keyWarn || unseen || !!settingsError });
}

function showAccount(open: boolean): void {
  $("p-open").classList.toggle("on", !open);
  $("p-account").classList.toggle("on", open);
  $("org").setAttribute("aria-expanded", open ? "true" : "false");
}

const HINT_SEEN = "consus.trayHintSeen";

// Once, after the first sign-in: where the launcher went.
function showHint(): void {
  if (localStorage.getItem(HINT_SEEN)) return;
  $("hintT").textContent = IS_WINDOWS
    ? "Consus lives in the taskbar's notification area. If you don't see the orange square, click ^ next to the clock and drag it onto the taskbar."
    : "Consus lives in your menu bar: the orange square. Click it any time to open a tool.";
  $("hint").style.display = "";
}

// A key revoked or expired since it was last checked, or Consus out of
// reach. Checked when the panel opens (at most every five minutes) and every
// three hours, so the orange square stays true.
let lastKeyCheck = 0;

async function recheckKey(force = false): Promise<void> {
  if (!K.def || (!force && Date.now() - lastKeyCheck < 5 * 60 * 1000)) return;
  lastKeyCheck = Date.now();
  const key = K.def.key;
  try {
    const result = await invoke<ConnectResult>("validate_key", { key });
    if (K.def?.key !== key) return;
    currentModels = result.models;
    keyWarn = false;
  } catch (err) {
    if (K.def?.key !== key) return;
    if ((err as ConnectError).kind === "Revoked") {
      await invoke("discard_dead_key").catch(() => undefined);
      signinMeta = null;
      K = { def: null };
      setConnected(false);
      $("signinErr").textContent = REVOKED_MESSAGE;
      return;
    }
    keyWarn = (err as ConnectError).kind === "Network";
  }
  renderStatus();
  syncTray();
}

// Sizes the window to what the panel shows, so a short view has no empty
// space; long lists still scroll. The visible scroll area is measured at its
// natural height, then put back; the observer forgets that change.
let fitTimer = 0;
let lastFit = 0;
let fitObserver: MutationObserver | null = null;

function fitPanel(): void {
  if (fitTimer) return;
  // A timer, not an animation frame: frames do not run while the panel is hidden.
  fitTimer = window.setTimeout(() => {
    fitTimer = 0;
    const panel = $("panel");
    const scroller = [$("connect"), $("p-open"), $("p-account")].find((el) => el.offsetParent !== null);
    if (!scroller) return;
    const [total, shown] = [panel.offsetHeight, scroller.offsetHeight];
    const prev = scroller.style.cssText;
    scroller.style.flex = "none";
    scroller.style.height = "auto";
    const natural = scroller.offsetHeight;
    scroller.style.cssText = prev;
    fitObserver?.takeRecords();
    const height = Math.ceil(total - shown + natural);
    if (Math.abs(height - lastFit) < 2) return;
    lastFit = height;
    void invoke("fit_panel", { height });
  }, 16);
}

function renderKeys(): void {
  if (!K.def) return;
  const m = signinMeta;
  const how =
    m?.source === "signin" ? "signed in" :
    m?.source === "helper" ? "signed in (credential helper)" :
    "pasted";
  const expiry = m?.expires ? " · expires " + esc(m.expires) : "";
  $("defRow").innerHTML =
    '<div class="t"><span>' + esc(K.def.mask) + "</span></div>" +
    '<div class="m">' + esc(K.def.user) + " · " + K.def.models + " models · " + how + expiry + "</div>" +
    '<div class="acts"><button class="mini" data-change="def">REPLACE</button></div><div id="edit-def"></div>';
  $("keyNote").textContent =
    m?.source === "signin" || m?.source === "helper"
      ? "Every tool uses this key. It lives in your system keychain, is yours alone, and expires on its own. Sign out revokes it."
      : "Every tool uses this key. It lives in your system keychain. A pasted key can only be revoked in the portal.";
}

function editor(slot: string): void {
  const box = $("edit-" + slot);
  if (box.innerHTML) {
    box.innerHTML = "";
    return;
  }
  box.innerHTML =
    '<input class="keyin" type="password" placeholder="Paste a key from portal.consus.io" id="in-' + slot + '" aria-label="New key">' +
    '<div style="display:flex;gap:6px;margin-top:8px">' +
    '<button class="mini" data-save="' + slot + '" style="border-color:var(--orange);color:#F3EBD6">SAVE</button>' +
    '<button class="mini" data-cancel="' + slot + '">CANCEL</button>' +
    '<button class="mini" data-portal="1">OPEN PORTAL ↗</button></div>' +
    '<div class="err" id="err-' + slot + '" role="alert"></div>';
  $("in-" + slot).focus();
}

async function resetAll(): Promise<void> {
  // Revokes the key and forgets it (keychain, record, an adopted helper copy).
  const r = await invoke<{ revoked: boolean; message: string }>("revoke_key").catch((err) => ({
    revoked: false,
    message: "Signed out on this computer. " + String(err),
  }));
  try {
    await invoke("remove_tool_configs");
  } catch {
    // best-effort; UI state still resets below
  }
  signinMeta = null;
  K = { def: null };
  $("signinErr").textContent = r.message;
  cur = null;
  keyWarn = false;
  document.querySelectorAll<HTMLElement>(".tool").forEach((x) => {
    x.classList.remove("on");
    x.querySelector(".go")!.textContent = "OPEN";
  });
  setConnected(false);
}

function signinErrorMessage(err: unknown): string {
  const e = err as SigninError;
  if (e?.kind === "Expired") return "That sign-in expired or was already used. Start again.";
  if (e?.kind === "Refused") return "The portal could not sign you in: " + e.message;
  if (e?.kind === "Network") return "Could not reach the portal. Check your connection. (" + e.message + ")";
  if (e?.kind === "NotStarted") return "";
  return String(err);
}

// Sign in through the portal: open the approval page, then ask (at the pace
// the portal allows) whether it was approved, until it was, failed, was
// cancelled, or ten minutes passed.
async function signIn(): Promise<void> {
  const attempt = ++signinAttempt;
  const mine = () => signinAttempt === attempt;
  const btn = $<HTMLButtonElement>("signinBtn");
  btn.disabled = true;
  $("signinErr").textContent = "";
  $("signinWait").querySelector("span")!.textContent = "Waiting for you to approve in the browser\u2026";
  $("signinWait").style.display = "";
  const deadline = Date.now() + SIGNIN_BUDGET_MS;
  try {
    const url = await invoke<string>("start_signin");
    await openUrl(url);
    for (;;) {
      if (!mine()) return;
      if (Date.now() > deadline) {
        void invoke("cancel_signin");
        $("signinErr").textContent = "No approval arrived in ten minutes. Start again when you are ready.";
        return;
      }
      const p = await invoke<{ meta: SigninMeta | null; retry_ms: number }>("poll_signin");
      if (!mine()) return;
      if (p.meta) {
        signinMeta = p.meta;
        // The key is issued; the gateway takes a few seconds to accept it.
        $("signinWait").querySelector("span")!.textContent = "Approved. Preparing your key, about 15 seconds\u2026";
        break;
      }
      await new Promise((r) => setTimeout(r, Math.max(1000, p.retry_ms)));
    }
    userConnected = true;
    const key = (await invoke<string | null>("keychain_get_key")) ?? "";
    const result = await validateWithPatience(key);
    K.def = applyConnectResult(key, result);
    lastKeyCheck = Date.now();
    setConnected(true);
    // The browser has the front by now; the panel takes it back.
    void invoke("show_panel");
  } catch (err) {
    if (mine()) $("signinErr").textContent = signinErrorMessage(err) || connectErrorMessage(err);
  } finally {
    if (mine()) {
      btn.disabled = false;
      $("signinWait").style.display = "none";
    }
  }
}

function cancelSignIn(): void {
  signinAttempt++;
  void invoke("cancel_signin");
  $("signinWait").style.display = "none";
  $<HTMLButtonElement>("signinBtn").disabled = false;
}

// Escape puts the panel away, as with any menu bar panel.
function wireEscape(): void {
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") void invoke("hide_panel");
  });
}

function wireConnect(): void {
  $("signinBtn").addEventListener("click", () => void signIn());
  $("signinCancel").addEventListener("click", cancelSignIn);
  $("openPortal").addEventListener("click", () => {
    void openUrl(PORTAL_URL);
    $("c1").className = "cn done";
    $("c1").textContent = "✓";
    $("c2").className = "cn done";
    $("c2").textContent = "✓";
    ($("keyIn") as HTMLInputElement).focus();
  });
  $("keyIn").addEventListener("input", () => {
    $("keyErr").textContent = "";
  });
  $("connectBtn").addEventListener("click", () => {
    const v = ($("keyIn") as HTMLInputElement).value;
    const e = validate(v);
    if (e) {
      $("keyErr").textContent = e;
      return;
    }
    const rawKey = v.trim();
    $("connectBtn").textContent = "CHECKING…";
    (async () => {
      const btn = $<HTMLButtonElement>("connectBtn");
      btn.disabled = true;
      userConnected = true;
      try {
        const result = await invoke<ConnectResult>("validate_key", { key: rawKey });
        signinMeta = await invoke<SigninMeta>("store_pasted_key", { key: rawKey, owner: result.email });
        K.def = applyConnectResult(rawKey, result);
        setConnected(true);
      } catch (err) {
        $("keyErr").textContent = connectErrorMessage(err);
      } finally {
        btn.disabled = false;
        btn.textContent = "CONNECT";
      }
    })();
  });
  $("keyIn").addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") $("connectBtn").click();
  });
}

function wireKeysTab(): void {
  $("p-account").addEventListener("click", (e) => {
    const t = e.target as HTMLElement;
    if (t.dataset.change) {
      editor(t.dataset.change);
      return;
    }
    if (t.dataset.cancel) {
      $("edit-" + t.dataset.cancel).innerHTML = "";
      return;
    }
    if (t.dataset.portal) {
      void openUrl(PORTAL_URL);
      return;
    }
    if (t.dataset.save) {
      const slot = t.dataset.save;
      const v = ($("in-" + slot) as HTMLInputElement).value;
      const er = validate(v);
      if (er) {
        $("err-" + slot).textContent = er;
        return;
      }
      const rawKey = v.trim();
      const btn = t as HTMLButtonElement;
      btn.textContent = "SAVING…";
      (async () => {
        btn.disabled = true;
        try {
          const result = await invoke<ConnectResult>("validate_key", { key: rawKey });
          signinMeta = await invoke<SigninMeta>("store_pasted_key", { key: rawKey, owner: result.email });
          K.def = applyConnectResult(rawKey, result);
          setConnected(true);
        } catch (err) {
          $("err-" + slot).textContent = connectErrorMessage(err);
          btn.disabled = false;
          btn.textContent = "SAVE";
        }
      })();
      return;
    }
    if (t.id === "signOut") void resetAll();
    if (t.id === "backBtn") showAccount(false);
  });
  $("p-account").addEventListener("input", (e) => {
    const id = (e.target as HTMLElement).id || "";
    if (id.indexOf("in-") === 0) $("err-" + id.slice(3)).textContent = "";
  });
}

function toolTile(key: Tool): HTMLButtonElement {
  return document.querySelector<HTMLButtonElement>(`.tool[data-app="${key}"]`)!;
}

interface ToolStatus {
  allowed: boolean;
  managed: boolean;
  installed: boolean;
  icon: string | null;
}

async function refreshTools(): Promise<void> {
  const status = await invoke<Record<string, ToolStatus>>("detect_tools");
  // Until the first answer, the list stays hidden: the org's settings and
  // what is installed decide which tiles show and how they look.
  $("p-open").classList.remove("pending");
  if (settingsError) {
    $("whoNote").textContent = settingsError;
  } else if (!TOOLS.some((k) => status[k]?.allowed)) {
    $("whoNote").textContent = "Your organization has not enabled any tools in the launcher.";
  }
  for (const key of TOOLS) {
    const st = status[key] ?? { allowed: false, managed: false, installed: false, icon: null };
    const b = toolTile(key);
    // Tools the org's settings leave out are not shown at all.
    b.style.display = st.allowed ? "" : "none";
    if (!st.allowed) continue;
    const go = b.querySelector<HTMLElement>(".go")!;
    const sb = b.querySelector<HTMLElement>("[data-sb]")!;
    const ic = b.querySelector<HTMLElement>(".ic")!;
    if (st.icon) {
      if (ic.querySelector("img")?.src !== st.icon) {
        const img = document.createElement("img");
        img.src = st.icon;
        img.alt = "";
        img.onerror = () => {
          ic.classList.remove("real");
          ic.textContent = ic.dataset.glyph ?? "";
        };
        ic.replaceChildren(img);
      }
      ic.classList.add("real");
    } else {
      ic.classList.remove("real");
      ic.textContent = ic.dataset.glyph ?? "";
    }
    if (st.installed) {
      delete b.dataset.missing;
      b.classList.remove("missing");
      if (go.textContent !== "RUNNING") go.textContent = "OPEN";
      // A machine-wide policy sets this tool up; the launcher just opens it.
      sb.textContent = st.managed ? "Managed by your organization" : (sb.dataset.d ?? "");
    } else {
      b.dataset.missing = "1";
      b.classList.add("missing");
      go.textContent = "GET IT ↗";
      sb.textContent = notInstalled(key);
    }
  }
  const ready = TOOLS.filter((k) => status[k]?.allowed && status[k]?.installed);
  void invoke("set_tray_menu", {
    tools: ready.map((k) => ({ key: k, name: TOOL_INFO[k].name })),
    update: updateVersion,
  });
}

function renderTiles(): void {
  $("toolList").replaceChildren(
    ...TOOLS.map((key) => {
      const t = TOOL_INFO[key];
      const b = document.createElement("button");
      b.className = "tool";
      b.dataset.app = key;
      b.innerHTML =
        '<span class="ic">' + esc(t.glyph) + "</span>" +
        '<span><div class="nm">' + esc(t.name) + '</div><div class="sb" data-sb="' + key + '">' + esc(t.sub) + "</div></span>" +
        '<span class="go">OPEN</span>';
      return b;
    }),
  );
}

function activateTile(b: HTMLButtonElement, key: Tool): void {
  document.querySelectorAll<HTMLButtonElement>(".tool").forEach((x) => {
    x.classList.remove("on");
    const k = x.dataset.app ?? "";
    const other = isTool(k) && x !== b && !running.has(k) && !x.dataset.missing;
    if (other) x.querySelector(".go")!.textContent = "OPEN";
  });
  b.classList.add("on");
  b.querySelector(".go")!.textContent = "RUNNING";
  cur = key;
}

async function launchTool(b: HTMLButtonElement, key: Tool): Promise<void> {
  if (b.disabled) return;
  const go = b.querySelector<HTMLElement>(".go")!;
  const sb = b.querySelector<HTMLElement>("[data-sb]")!;
  b.disabled = true;
  go.textContent = "OPENING…";
  // Cleared before the launch, not after: a notice sent during it must stay.
  sb.textContent = sb.dataset.d ?? "";
  try {
    await invoke("launch_tool", { tool: key, models: currentModels ?? [] });
    running.add(key);
    activateTile(b, key);
    void invoke("hide_panel");
  } catch (err) {
    go.textContent = "OPEN";
    sb.textContent = String(err);
  } finally {
    b.disabled = false;
  }
}

function wireTools(): void {
  document.querySelectorAll<HTMLButtonElement>(".tool").forEach((b) => {
    b.addEventListener("click", () => {
      const a = b.dataset.app ?? "";
      if (!isTool(a)) return;
      if (b.dataset.missing) {
        void openUrl(TOOL_INFO[a].download);
        b.querySelector("[data-sb]")!.textContent = "Download opened in your browser. Install it, then come back.";
        return;
      }
      void launchTool(b, a);
    });
  });
}

function wireAccount(): void {
  $("org").addEventListener("click", () => showAccount(!$("p-account").classList.contains("on")));
  $("hintOk").addEventListener("click", () => {
    localStorage.setItem(HINT_SEEN, "1");
    $("hint").style.display = "none";
  });
}

async function restoreSession(): Promise<void> {
  // A key the credential helper script already obtained on this machine
  // is adopted, so nobody signs in twice.
  let storedKey = await invoke<string | null>("keychain_get_key");
  if (!storedKey) storedKey = await invoke<string | null>("adopt_helper_key").catch(() => null);
  if (!storedKey || userConnected) return;
  signinMeta = await invoke<SigninMeta | null>("signin_info").catch(() => null);
  let result: ConnectResult;
  try {
    result = await invoke<ConnectResult>("validate_key", { key: storedKey });
  } catch (err) {
    // If the user connected with a new key while this was in flight, the
    // stored key is no longer the one we validated, so leave it alone.
    if (userConnected) return;
    if ((err as ConnectError).kind === "Revoked") {
      // Forgotten for good, the helper script's copy included, so it is
      // not adopted again at the next start.
      await invoke("discard_dead_key").catch(() => undefined);
      signinMeta = null;
      $("signinErr").textContent = REVOKED_MESSAGE;
    } else if (settingsError) {
      $("signinErr").textContent = settingsError;
    }
    // Network/Rejected failures at startup: leave the stored key alone and
    // stay on first-run rather than guess at a transient-vs-permanent error.
    return;
  }
  if (userConnected) return;
  K.def = applyConnectResult(storedKey, result);
  lastKeyCheck = Date.now();
  setConnected(true);
}

interface Update {
  version: string;
  url: string;
}

// The portal's note of the newest launcher, read at start and twice a day.
// The launcher never updates itself: this only points at the download.
async function checkUpdate(): Promise<void> {
  const u = await invoke<Update | null>("check_update").catch(() => null);
  const b = $("update");
  if (!u) {
    b.style.display = "none";
    updateVersion = null;
    return;
  }
  if (u.version !== updateVersion) unseen = true;
  updateVersion = u.version;
  updateUrl = u.url;
  b.textContent = `Consus Launcher ${u.version} is available. Download ↗`;
  b.onclick = () => void openUrl(u.url);
  b.style.display = "";
  syncTray();
  // The icon's menu carries the download too.
  if (K.def) void refreshTools();
  else void invoke("set_tray_menu", { tools: [], update: updateVersion });
}

let updateUrl: string | null = null;

function init(): void {
  renderTiles();
  document.querySelectorAll<HTMLElement>(".tool [data-sb]").forEach((el) => {
    el.dataset.d = el.textContent ?? "";
  });
  document.querySelectorAll<HTMLElement>(".tool .ic").forEach((ic) => {
    ic.dataset.glyph = ic.textContent ?? "";
  });
  wireEscape();
  wireConnect();
  wireKeysTab();
  wireTools();
  wireAccount();
  fitObserver = new MutationObserver(fitPanel);
  fitObserver.observe($("panel"), { subtree: true, childList: true, characterData: true, attributes: true });
  fitPanel();
  void listen<string>("tray-tool", (e) => {
    if (isTool(e.payload) && K.def) void launchTool(toolTile(e.payload), e.payload);
  });
  void listen("tray-update", () => {
    if (updateUrl) void openUrl(updateUrl);
  });
  setInterval(() => void recheckKey(true), 3 * 60 * 60 * 1000);
  void listen<string>("tool-exited", (e) => {
    if (!isTool(e.payload)) return;
    running.delete(e.payload);
    const b = toolTile(e.payload);
    b.classList.remove("on");
    b.querySelector(".go")!.textContent = "OPEN";
    if (cur === e.payload) {
      const next = [...running][0] ?? null;
      cur = next;
      if (next) toolTile(next).classList.add("on");
    }
    void refreshTools();
  });
  void listen<{ tool: string; message: string }>("tool-notice", (e) => {
    if (!isTool(e.payload.tool)) return;
    const sb = toolTile(e.payload.tool).querySelector<HTMLElement>("[data-sb]")!;
    sb.textContent = e.payload.message;
    if (!document.hasFocus()) {
      unseen = true;
      syncTray();
    }
  });
  void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    if (!focused) return;
    if (unseen) {
      unseen = false;
      syncTray();
    }
    if (K.def) {
      void refreshTools();
      void recheckKey();
    }
  });
  void checkUpdate();
  setInterval(() => void checkUpdate(), 12 * 60 * 60 * 1000);
  void loadSettings()
    .catch(() => undefined)
    .then(restoreSession);
}

init();
