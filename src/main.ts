import { getCurrentWindow } from "@tauri-apps/api/window";

type ToolKey = "desktop" | "chatgpt" | "code" | "vscode";

interface KeyInfo {
  key: string;
  mask: string;
  user: string;
  models: number;
  added: string;
}

interface LogRow {
  t: string;
  host: string;
  kb: number;
  app: string;
  m: string;
  k: string;
  ms: number;
}

const TOOLS: Record<ToolKey, [string, string]> = {
  desktop: ["Claude Desktop", "claude-sonnet-5:itar"],
  chatgpt: ["ChatGPT", "gpt-5.6-terra:itar"],
  code: ["Claude Code", "claude-sonnet-5:itar"],
  vscode: ["VS Code", "gpt-5.6-terra:cui"],
};

interface KeyStore {
  def: KeyInfo | null;
  tool: Partial<Record<ToolKey, KeyInfo>>;
}

let cur: ToolKey | null = null;
let rows: LogRow[] = [];
let hosts: Record<string, 1> = {};
let K: KeyStore = { def: null, tool: {} };
let timer: ReturnType<typeof setInterval> | null = null;
const last: Partial<Record<ToolKey, number>> = {};

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

function ts(): string {
  return new Date().toTimeString().slice(0, 8);
}

function mask(k: string): string {
  return "csk_…" + k.slice(-4);
}

function validate(k: string): string | null {
  k = (k || "").trim();
  if (!k) return "Paste a key first.";
  if (k.indexOf("csk_") !== 0) return "That doesn't look like a Consus key. They start with csk_.";
  if (k.length < 16) return "That key looks cut off. Copy it again from the portal.";
  return null;
}

function info(k: string): KeyInfo {
  return { key: k, mask: mask(k), user: "eric@company.com", models: 6, added: new Date().toLocaleDateString() };
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
  } else {
    ($("keyIn") as HTMLInputElement).value = "";
    $("keyErr").textContent = "";
    (["c1", "c2", "c3"] as const).forEach((c) => ($(c).className = "cn"));
    $("c1").textContent = "1";
    $("c2").textContent = "2";
    $("c3").textContent = "3";
  }
}

function keyFor(a: ToolKey): KeyInfo {
  return K.tool[a] ?? K.def!;
}

function renderKeys(): void {
  if (!K.def) return;
  $("defRow").innerHTML =
    '<div class="t"><span>' + esc(K.def.mask) + '</span><span class="tag own">DEFAULT</span></div>' +
    '<div class="m">' + esc(K.def.user) + " · " + K.def.models + " models · added " + K.def.added + "</div>" +
    '<div class="acts"><button class="mini" data-change="def">REPLACE</button></div><div id="edit-def"></div>';
  $("toolKeys").innerHTML = (Object.keys(TOOLS) as ToolKey[])
    .map((a) => {
      const own = !!K.tool[a];
      const k = keyFor(a);
      return (
        '<div class="krow"><div class="t"><span>' + TOOLS[a][0] + '</span><span class="tag' + (own ? " own" : "") + '">' +
        (own ? "OWN KEY" : "DEFAULT") + '</span></div><div class="m">' + esc(k.mask) + '</div><div class="acts">' +
        '<button class="mini" data-change="' + a + '">' + (own ? "REPLACE" : "USE A DIFFERENT KEY") + "</button>" +
        (own ? '<button class="mini" data-reset="' + a + '">USE DEFAULT</button>' : "") +
        '</div><div id="edit-' + a + '"></div></div>'
      );
    })
    .join("");
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

function pad(x: string | number, n: number): string {
  let s = String(x);
  while (s.length < n) s += " ";
  return s;
}

function render(): void {
  const n = Object.keys(hosts).length;
  $("nDest").textContent = String(n);
  $("pl").textContent = n === 1 ? "" : "s";
  $("tcount").textContent = rows.length ? "· " + rows.length : "";
  if (!rows.length) {
    $("feed").innerHTML = '<span class="dim">// waiting for the first request</span>\n';
    return;
  }
  $("feed").innerHTML = rows
    .slice(0, 60)
    .map(
      (r) =>
        '<span class="l"><span class="t">' + r.t + '</span>  <span class="h">' + pad(r.host, 16) + "</span> " +
        pad(r.kb + " kB", 7) + " " + pad(r.app, 15) + " " + pad(r.m, 22) + ' <span class="k">' + esc(r.k) + "</span>  " + r.ms + " ms</span>",
    )
    .join("");
  $("feed").scrollTop = 0;
}

function stream(): void {
  if (timer) clearInterval(timer);
  if (!cur) return;
  timer = setInterval(() => {
    if (Math.random() < 0.55) req();
  }, 1800);
}

function ago(a: ToolKey): string | null {
  const t = last[a];
  if (!t) return null;
  const m = Math.round((Date.now() - t) / 60000);
  return m < 1 ? "just now" : m + " min ago";
}

function subs(): void {
  (Object.keys(TOOLS) as ToolKey[]).forEach((a) => {
    const el = document.querySelector<HTMLElement>('[data-sb="' + a + '"]');
    if (!el || el.closest(".tool")?.hasAttribute("data-missing")) return;
    const t = ago(a);
    el.textContent = t ? "Last used " + t : el.dataset.d ?? "";
  });
}

function req(): void {
  if (!cur) return;
  last[cur] = Date.now();
  subs();
  hosts["api.consus.ai"] = 1;
  rows.unshift({
    t: ts(),
    host: "api.consus.ai",
    kb: Math.round(Math.random() * 38 + 3),
    app: TOOLS[cur][0],
    m: TOOLS[cur][1],
    k: keyFor(cur).mask,
    ms: Math.round(Math.random() * 900 + 300),
  });
  $("rate").textContent = "live";
  render();
}

function resetAll(): void {
  K = { def: null, tool: {} };
  cur = null;
  rows = [];
  hosts = {};
  for (const k of Object.keys(last) as ToolKey[]) delete last[k];
  if (timer) clearInterval(timer);
  $("rate").textContent = "idle";
  render();
  document.querySelectorAll<HTMLElement>(".appwin").forEach((w) => {
    w.classList.remove("on");
    w.style.display = "";
  });
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
    $("connectBtn").textContent = "CHECKING…";
    setTimeout(() => {
      $("connectBtn").textContent = "CONNECT";
      K.def = info(v.trim());
      setConnected(true);
    }, 600);
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
    if (t.dataset.portal) return;
    if (t.id === "adv") {
      const b = $("advBox");
      const o = b.style.display === "none";
      b.style.display = o ? "block" : "none";
      t.setAttribute("aria-expanded", o ? "true" : "false");
      t.textContent = o ? "HIDE PER-TOOL KEYS" : "ADVANCED: ONE KEY PER TOOL";
      return;
    }
    if (t.dataset.reset) {
      delete K.tool[t.dataset.reset as ToolKey];
      renderKeys();
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
      const k = info(v.trim());
      if (slot === "def") K.def = k;
      else K.tool[slot as ToolKey] = k;
      renderKeys();
      return;
    }
    if (t.id === "signOut") resetAll();
  });
  $("p-keys").addEventListener("input", (e) => {
    const id = (e.target as HTMLElement).id || "";
    if (id.indexOf("in-") === 0) $("err-" + id.slice(3)).textContent = "";
  });
}

function wireTools(): void {
  document.querySelectorAll<HTMLButtonElement>(".tool").forEach((b) => {
    b.addEventListener("click", () => {
      const a = b.dataset.app as ToolKey;
      if (b.dataset.missing) {
        b.querySelector(".go")!.textContent = "OPENING…";
        b.querySelector("[data-sb]")!.textContent = "Download opened in your browser. Install it, then come back.";
        setTimeout(() => {
          delete b.dataset.missing;
          b.classList.remove("missing");
          b.querySelector(".go")!.textContent = "OPEN";
          const sb = b.querySelector<HTMLElement>("[data-sb]")!;
          sb.textContent = "Installed. Config written.";
          sb.dataset.d = "Editor chat";
          setTimeout(subs, 2500);
        }, 3500);
        return;
      }
      document.querySelectorAll<HTMLElement>(".tool").forEach((x) => {
        x.classList.remove("on");
        x.querySelector(".go")!.textContent = "OPEN";
      });
      b.classList.add("on");
      b.querySelector(".go")!.textContent = "RUNNING";
      cur = a;
      $("empty").style.display = "none";
      document.querySelectorAll<HTMLElement>(".appwin").forEach((w) => {
        w.classList.remove("on");
        w.style.display = "";
      });
      const w = $("w-" + a);
      w.style.display = "flex";
      requestAnimationFrame(() => w.classList.add("on"));
      req();
      stream();
    });
  });
}

function wireSend(): void {
  document.querySelectorAll<HTMLButtonElement>("[data-send]").forEach((b) => {
    b.addEventListener("click", () => {
      req();
      setTimeout(req, 450);
      if (cur === "code") $("codeOut").textContent = "  Reading sim/thermal.py … explaining lumped-mass model";
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
      (["open", "keys", "traffic"] as const).forEach((n) => $("p-" + n).classList.toggle("on", b.dataset.tab === n));
    });
  });
}

function init(): void {
  document.querySelectorAll<HTMLElement>("[data-sb]").forEach((el) => {
    if (!el.closest(".tool")?.hasAttribute("data-missing")) el.dataset.d = el.textContent ?? "";
  });
  setInterval(subs, 30000);
  wireWindowControls();
  wireConnect();
  wireKeysTab();
  wireTools();
  wireSend();
  wireTabs();
}

init();
