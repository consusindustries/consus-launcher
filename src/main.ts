import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";

const PORTAL_URL = "https://portal.consus.io";
// Tools the launcher detects, configures, and launches.
type Tool = "desktop" | "chatgpt" | "code" | "codex" | "pi";
const TOOLS: Tool[] = ["desktop", "chatgpt", "code", "codex", "pi"];
const DOWNLOAD: Record<Tool, string> = {
  desktop: "https://claude.ai/download",
  chatgpt: "https://openai.com/chatgpt/download/",
  code: "https://claude.com/claude-code",
  codex: "https://developers.openai.com/codex",
  pi: "https://pi.dev",
};
const NOT_INSTALLED: Record<Tool, string> = {
  desktop: "Not installed · get it from claude.ai/download",
  chatgpt: "Not installed · get it from openai.com",
  code: "Not installed · get it from claude.com/claude-code",
  codex: "Not installed · get it from developers.openai.com/codex",
  pi: "Not installed · get it from pi.dev",
};
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

const REVOKED_MESSAGE = "This key was revoked. Paste a new one from the portal.";

// Portal commands reject with a ConnectError object; keychain commands reject with a string.
function connectErrorMessage(err: unknown): string {
  if (typeof err === "string") return "The portal accepted the key, but it could not be saved to your keychain.";
  if ((err as ConnectError).kind === "Network") return "Could not reach the portal. Check your connection.";
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
}

async function loadSettings(): Promise<void> {
  const s = await invoke<SettingsView>("get_settings");
  $("orgName").textContent = s.org_name ?? "Consus";
  settingsError = s.error ? s.error + (s.managed ? " Contact your IT admin." : "") : null;
  if (settingsError) {
    $("empty").querySelector("b")!.textContent = "Launcher settings need attention.";
    $("empty").querySelector("span")!.textContent = settingsError;
  }
}

function applyConnectResult(rawKey: string, result: ConnectResult): KeyInfo {
  currentModels = result.models;
  return {
    key: rawKey,
    mask: mask(rawKey),
    user: result.email ?? mask(rawKey),
    models: result.model_count,
    added: new Date().toLocaleDateString(),
  };
}

function setConnected(on: boolean): void {
  $("connect").style.display = on ? "none" : "block";
  $("main").style.display = on ? "flex" : "none";
  $("status").classList.toggle("off", !on);
  $("statusT").textContent = on ? "CONNECTED" : "NOT CONNECTED";
  if (on && K.def) {
    $("empty").querySelector("b")!.textContent = "Pick a tool.";
    $("empty").querySelector("span")!.textContent = "It opens here, set up for Consus.";
    $("whoNote").textContent =
      "Signed in as " + K.def.user + ". Every tool here uses Consus. Anything not installed links to the vendor; Consus never installs software.";
    renderKeys();
    void refreshTools();
  } else {
    ($("keyIn") as HTMLInputElement).value = "";
    $("keyErr").textContent = "";
    (["c1", "c2", "c3"] as const).forEach((c) => ($(c).className = "cn"));
    $("c1").textContent = "1";
    $("c2").textContent = "2";
    $("c3").textContent = "3";
  }
}

function renderKeys(): void {
  if (!K.def) return;
  $("defRow").innerHTML =
    '<div class="t"><span>' + esc(K.def.mask) + "</span></div>" +
    '<div class="m">' + esc(K.def.user) + " · " + K.def.models + " models · added " + K.def.added + "</div>" +
    '<div class="acts"><button class="mini" data-change="def">REPLACE</button></div><div id="edit-def"></div>';
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
  for (const cmd of ["keychain_delete_key", "remove_tool_configs"]) {
    try {
      await invoke(cmd);
    } catch {
      // best-effort; UI state still resets below
    }
  }
  K = { def: null };
  cur = null;
  $("empty").style.display = "";
  document.querySelectorAll<HTMLElement>(".tool").forEach((x) => {
    x.classList.remove("on");
    x.querySelector(".go")!.textContent = "OPEN";
  });
  $("empty").querySelector("b")!.textContent = "Connect, then pick a tool.";
  $("empty").querySelector("span")!.textContent = "Paste your key on the right. One time.";
  setConnected(false);
}

function wireWindowControls(): void {
  const win = getCurrentWindow();
  document.querySelectorAll<HTMLElement>(".chrome .lights i[data-action]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const action = btn.dataset.action;
      if (action === "close") void win.close();
      else if (action === "minimize") void win.minimize();
      else if (action === "maximize") void win.toggleMaximize();
    });
  });
  $("chrome").addEventListener("dblclick", (e) => {
    if ((e.target as HTMLElement).closest(".lights")) return;
    void win.toggleMaximize();
  });
}

function wireConnect(): void {
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
        await invoke("keychain_set_key", { key: rawKey });
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
  $("p-keys").addEventListener("click", (e) => {
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
          await invoke("keychain_set_key", { key: rawKey });
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
  });
  $("p-keys").addEventListener("input", (e) => {
    const id = (e.target as HTMLElement).id || "";
    if (id.indexOf("in-") === 0) $("err-" + id.slice(3)).textContent = "";
  });
}

function toolTile(key: Tool): HTMLButtonElement {
  return document.querySelector<HTMLButtonElement>(`.tool[data-app="${key}"]`)!;
}

interface ToolStatus {
  allowed: boolean;
  installed: boolean;
  icon: string | null;
}

async function refreshTools(): Promise<void> {
  const status = await invoke<Record<string, ToolStatus>>("detect_tools");
  if (settingsError) {
    $("whoNote").textContent = settingsError;
  } else if (!TOOLS.some((k) => status[k]?.allowed)) {
    $("whoNote").textContent = "Your organization has not enabled any tools in the launcher.";
  }
  for (const key of TOOLS) {
    const st = status[key] ?? { allowed: false, installed: false, icon: null };
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
      sb.textContent = sb.dataset.d ?? "";
    } else {
      b.dataset.missing = "1";
      b.classList.add("missing");
      go.textContent = "GET IT ↗";
      sb.textContent = NOT_INSTALLED[key];
    }
  }
}

// The glass rect in screen points, where a launched app's window goes.
async function glassRect(): Promise<{ x: number; y: number; w: number; h: number }> {
  const win = getCurrentWindow();
  const [pos, scale] = await Promise.all([win.outerPosition(), win.scaleFactor()]);
  const r = $("glass").getBoundingClientRect();
  const inset = 16;
  return {
    x: pos.x / scale + r.left + inset,
    y: pos.y / scale + r.top + inset,
    w: r.width - 2 * inset,
    h: r.height - 2 * inset,
  };
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
  $("empty").style.display = "none";
}

async function launchTool(b: HTMLButtonElement, key: Tool): Promise<void> {
  if (b.disabled) return;
  const go = b.querySelector<HTMLElement>(".go")!;
  const sb = b.querySelector<HTMLElement>("[data-sb]")!;
  b.disabled = true;
  go.textContent = "OPENING…";
  try {
    await invoke("launch_tool", { tool: key, rect: await glassRect(), models: currentModels ?? [] });
    running.add(key);
    activateTile(b, key);
    sb.textContent = sb.dataset.d ?? "";
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
        void openUrl(DOWNLOAD[a]);
        b.querySelector("[data-sb]")!.textContent = "Download opened in your browser. Install it, then come back.";
        return;
      }
      void launchTool(b, a);
    });
  });
}

function wireTabs(): void {
  document.querySelectorAll<HTMLButtonElement>(".tabs button").forEach((b) => {
    b.addEventListener("click", () => {
      document.querySelectorAll<HTMLButtonElement>(".tabs button").forEach((x) => {
        x.classList.toggle("on", x === b);
        x.setAttribute("aria-selected", x === b ? "true" : "false");
      });
      (["open", "keys"] as const).forEach((n) => $("p-" + n).classList.toggle("on", b.dataset.tab === n));
    });
  });
}

async function restoreSession(): Promise<void> {
  const storedKey = await invoke<string | null>("keychain_get_key");
  if (!storedKey || userConnected) return;
  let result: ConnectResult;
  try {
    result = await invoke<ConnectResult>("validate_key", { key: storedKey });
  } catch (err) {
    // If the user connected with a new key while this was in flight, the
    // stored key is no longer the one we validated, so leave it alone.
    if (userConnected) return;
    if ((err as ConnectError).kind === "Revoked") {
      try {
        await invoke("keychain_delete_key");
      } catch {
        // the next successful connect overwrites it anyway
      }
      $("keyErr").textContent = REVOKED_MESSAGE;
    } else if (settingsError) {
      $("keyErr").textContent = settingsError;
    }
    // Network/Rejected failures at startup: leave the stored key alone and
    // stay on first-run rather than guess at a transient-vs-permanent error.
    return;
  }
  if (userConnected) return;
  K.def = applyConnectResult(storedKey, result);
  setConnected(true);
}

function init(): void {
  document.querySelectorAll<HTMLElement>(".tool [data-sb]").forEach((el) => {
    el.dataset.d = el.textContent ?? "";
  });
  document.querySelectorAll<HTMLElement>(".tool .ic").forEach((ic) => {
    ic.dataset.glyph = ic.textContent ?? "";
  });
  wireWindowControls();
  wireConnect();
  wireKeysTab();
  wireTools();
  wireTabs();
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
    if (running.size === 0) $("empty").style.display = "";
    void refreshTools();
  });
  void listen<{ tool: string; message: string }>("tool-notice", (e) => {
    if (!isTool(e.payload.tool)) return;
    const sb = toolTile(e.payload.tool).querySelector<HTMLElement>("[data-sb]")!;
    sb.textContent = e.payload.message;
  });
  void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    if (focused && K.def) void refreshTools();
  });
  void loadSettings()
    .catch(() => undefined)
    .then(restoreSession);
}

init();
