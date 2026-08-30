// Separate file, not an inline <script>, so the CSP can stay `script-src 'self'`.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const treeEl = $("tree"), listEl = $("list"), detailEl = $("d-body");
const searchBtn = $("searchbtn");
const paletteEl = $("palette"), pq = $("pq"), presultsEl = $("presults");
const ctxEl = $("ctx");
const sheetWrap = $("sheetwrap"), jackForm = $("jackform"), jfErr = $("jf-err"), jfDelete = $("jf-delete");
const askWrap = $("askwrap"), askForm = $("askform"), askInput = $("ask-input"), askErr = $("ask-err");

const isMac = navigator.userAgent.includes("Mac");
if (isMac) document.body.dataset.os = "macos";
// Shortcut labels: ⌘K on macOS, Ctrl+K everywhere else. The handler already accepts both.
const chord = (k) => (isMac ? `⌘${k.toUpperCase()}` : `Ctrl+${k.toUpperCase()}`);

let all = [];                 // every jack, in file order
let shown = [];               // what the middle column currently lists
let probes = new Map();       // name -> { target, ms }
let group = null;             // selected tag path, null = all
let expanded = new Set();     // open tag paths
let sel = 0;
let palSel = 0;
let editing = null;   // jack name being edited, or null when adding
// Folders only exist because a jack carries the tag, so a brand-new empty one is
// held here until something lands in it. Dropped on reload, which is honest.
let pending = new Set();
let seeded = false;           // the tree's initial expansion is a one-off
let lastProbe = 0;            // epoch ms of the last sweep, for the throttle below

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

// Lucide, inlined at generate time by scripts/icons.mjs — see ui/icons.js.
const icon = (name) =>
  `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
     stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] ?? ""}</svg>`;

// `os = "..."` in the config, matched loosely so "Ubuntu 22.04" and "ubuntu" agree.
// Brand marks are fill paths (simple-icons); the fallbacks are stroked Lucide shapes.
const osKey = (os) => (os ?? "").toLowerCase().replace(/[^a-z]/g, "");
function osIcon(os) {
  const k = osKey(os);
  const brand = Object.keys(BRANDS).find((b) => k.startsWith(b) || (b.startsWith(k) && k.length > 2));
  if (k && brand) {
    return `<svg class="i brand" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="${BRANDS[brand]}"/></svg>`;
  }
  const fb = Object.keys(BRAND_FALLBACKS).find((b) => k === b);
  return icon(fb ? BRAND_FALLBACKS[fb] : "server");
}
const hit = (j, f) =>
  !f || j.name.toLowerCase().includes(f) || j.host.toLowerCase().includes(f) ||
  (j.desc ?? "").toLowerCase().includes(f) || j.tags.some((t) => t.toLowerCase().includes(f));

// ── sidebar tree ───────────────────────────────────────────────────────────
// A tag of "prod/eu/web" nests three deep; a jack counts toward every ancestor,
// and toward more than one branch if it carries more than one tag.
function buildTree() {
  const root = new Map();
  const tagged = all.flatMap((j) => j.tags.map((t) => [j, t]));
  for (const p of pending) tagged.push([null, p]);
  for (const [j, tag] of tagged) {
    {
      let level = root, path = "";
      for (const part of tag.split("/").map((p) => p.trim()).filter(Boolean)) {
        path = path ? `${path}/${part}` : part;
        if (!level.has(part)) level.set(part, { name: part, path, members: new Set(), children: new Map() });
        const node = level.get(part);
        if (j) node.members.add(j.name);
        level = node.children;
      }
    }
  }
  return root;
}

function renderTree() {
  const rows = [`<div class="side-title">Patchbay</div>`,
    row({ name: "All jacks", path: null, members: new Set(all.map((j) => j.name)), children: new Map() }, 0, "layers")];

  const walk = (level, depth) => {
    for (const node of [...level.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      rows.push(row(node, depth));
      if (expanded.has(node.path)) walk(node.children, depth + 1);
    }
  };
  const tree = buildTree();
  if (tree.size) rows.push(`<div class="side-title">Groups</div>`);
  walk(tree, 0);

  const untagged = all.filter((j) => !j.tags.length).length;
  if (untagged) rows.push(row({ name: "Untagged", path: "\0untagged", members: new Set(Array(untagged)), children: new Map() }, 0, "circle-off"));
  treeEl.innerHTML = rows.join("");
}

function row(node, depth, glyph) {
  const kids = node.children.size > 0;
  const open = expanded.has(node.path);
  const g = glyph ?? (kids ? (open ? "folder-open" : "folder") : "tag");
  return `<div class="group" data-path="${esc(node.path ?? "")}" data-has-kids="${kids}"
       aria-current="${group === node.path}" style="padding-left:${8 + depth * 13}px">
    <span class="twist ${kids ? "" : "leaf"} ${open ? "open" : ""}">${icon("chevron-right")}</span>
    <span class="gi">${icon(g)}</span>
    <span class="label">${esc(node.name)}</span>
    <span class="n">${node.members.size}</span>
  </div>`;
}

const inGroup = (j) =>
  group === null ? true
  : group === "\0untagged" ? j.tags.length === 0
  : j.tags.some((t) => t === group || t.startsWith(group + "/"));

// ── list ───────────────────────────────────────────────────────────────────
function render() {
  renderTree();
  shown = all.filter(inGroup);

  renderTabs();
  searchBtn.innerHTML = `${icon("search")}Search<kbd>${chord("k")}</kbd>`;
  $("newjack").innerHTML = `${icon("plus")}Device<kbd>${chord("n")}</kbd>`;
  $("newgroup").innerHTML = icon("folder-plus");
  $("newgroup").title = "New folder";
  $("editcfg").innerHTML = icon("file-pen-line");
  $("editcfg").title = `Open the config file (${chord("e")})`;

  if (!shown.length) {
    listEl.innerHTML = `<p class="empty">${all.length ? "nothing here" : "no jacks yet — <code>bay edit</code>"}</p>`;
    renderDetail();   // a session tab still has something to describe
    return;
  }
  sel = Math.min(sel, shown.length - 1);
  listEl.innerHTML = shown.map((j, i) => {
    const p = probes.get(j.name);
    const state = !p ? "unknown" : p.ms == null ? "down" : "up";
    return `<div class="jack" data-i="${i}" aria-selected="${i === sel}">
      <span class="dot ${state}"></span>
      <span class="os" title="${esc(j.os ?? "")}">${osIcon(j.os)}</span>
      <span class="name">${esc(j.name)}</span>
      <span class="host">${esc(j.user ? j.user + "@" + j.host : j.host)}${j.port ? ":" + j.port : ""}</span>
      <span class="tags">${j.tags.map((t) => `<span class="tag">${esc(t.split("/").pop())}</span>`).join("")}</span>
    </div>`;
  }).join("");
  renderDetail();
  listEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

function renderDetail() {
  // On a session tab the pane describes that session's jack, not the list selection.
  const live = activeId !== null ? sessions.get(activeId) : null;
  const j = live ? all.find((x) => x.name === live.name) : shown[sel];
  if (!j) return (detailEl.innerHTML = "");
  const p = probes.get(j.name);
  const reach = !p ? `<span style="color:var(--fg-faint)">checking…</span>`
    : p.ms == null ? `<span style="color:var(--down)">no answer</span> · ${esc(p.target)}`
    : `<span style="color:var(--up)">up</span> · ${esc(p.target)} · ${p.ms}ms`;

  const stops = [...j.hops, j.user ? `${j.user}@${j.host}` : j.host];
  detailEl.innerHTML = `
    <div class="d-name"><span class="d-os">${osIcon(j.os)}</span>${esc(j.name)}</div>
    ${j.desc ? `<div class="d-desc">${esc(j.desc)}</div>` : `<div class="d-desc"></div>`}

    <div class="d-sec">${icon("server")}Target</div>
    <dl>
      <div class="d-row"><dt>host</dt><dd>${esc(j.host)}</dd></div>
      ${j.user ? `<div class="d-row"><dt>user</dt><dd>${esc(j.user)}</dd></div>` : ""}
      ${j.port ? `<div class="d-row"><dt>port</dt><dd>${j.port}</dd></div>` : ""}
      ${j.key ? `<div class="d-row"><dt>key</dt><dd>${esc(j.key)}</dd></div>` : ""}
    </dl>

    <div class="d-sec">${icon("waypoints")}Route</div>
    <div class="route">
      <span><i class="pip"></i>this machine</span>
      ${stops.map((h, i) => `<span class="${i === stops.length - 1 ? "last" : ""}">
        <i class="pip"></i>${esc(h)}${i < stops.length - 1 ? `<i class="arm">jump</i>` : ""}</span>`).join("")}
    </div>

    ${j.forward.length ? `<div class="d-sec">${icon("arrow-right-left")}Forwards</div>
      <dl>${j.forward.map((f) => `<div class="d-row"><dt>-L</dt><dd>${esc(f)}</dd></div>`).join("")}</dl>` : ""}

    <div class="d-sec">${icon("plug")}Reachable</div>
    <div style="font-size:12.5px">${reach}</div>

    <div class="d-sec">${icon("square-terminal")}Command</div>
    <div class="mono ${j.command.startsWith("ssh ") ? "" : "err"}">${esc(j.command)}</div>
    <div class="btns">
      ${live
        ? `<button class="primary" data-act="disconnect">${icon("x")}${live.dead ? "Close tab" : "Disconnect"}</button>`
        : `<button class="primary" data-act="connect">${icon("square-terminal")}Connect</button>`}
      <button class="ghost" data-act="edit" title="Edit">${icon("pencil")}</button>
      <button class="ghost" data-act="copy" title="Copy command">${icon("copy")}</button>
    </div>`;
}

function move(d) {
  if (!shown.length) return;
  sel = (sel + d + shown.length) % shown.length;
  render();
}

// In-app by default; `inTerminal` hands off to Terminal.app / wt / gnome-terminal.
async function connect(name, inTerminal = false) {
  if (!inTerminal) return openSession(name);
  try {
    await invoke("connect", { name });
  } catch (e) {
    alertish(e);
  }
}

// ── command palette ────────────────────────────────────────────────────────
const palOpen = () => !paletteEl.hidden;
// `seed` is set when you just start typing in the list — the keystroke isn't lost.
function openPalette(seed = "") {
  paletteEl.hidden = false;
  pq.value = seed; palSel = 0;
  $("pq-icon").innerHTML = icon("search");
  renderPalette();
  pq.focus();
  pq.setSelectionRange(seed.length, seed.length);
}
function closePalette() { paletteEl.hidden = true; }

function palMatches() {
  const f = pq.value.trim().toLowerCase();
  return all.filter((j) => hit(j, f)).slice(0, 40);
}
function renderPalette() {
  const rows = palMatches();
  palSel = Math.min(palSel, Math.max(0, rows.length - 1));
  presultsEl.innerHTML = rows.length
    ? rows.map((j, i) => `<div class="jack" data-pi="${i}" aria-selected="${i === palSel}">
        <span class="name">${esc(j.name)}</span>
        <span class="host">${esc(j.host)}</span></div>`).join("")
    : `<p class="empty">no match</p>`;
  presultsEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

// ── sessions ───────────────────────────────────────────────────────────────
// One xterm per session, all stacked in #terms with only the active one shown.
const { listen } = window.__TAURI__.event;
const tabsEl = $("tabs"), termsEl = $("terms"), browseEl = $("browse");
const sessions = new Map();   // id -> { id, name, term, fit, host, dead, unlisten[] }
let activeId = null;
let nextId = 1;

const theme = () => {
  const css = getComputedStyle(document.body);
  const v = (name, fallback) => css.getPropertyValue(name).trim() || fallback;
  return {
    background: "rgba(0,0,0,0)",
    foreground: v("--fg", "#f0f0f4"),
    cursor: v("--accent", "#4f9dfd"),
    selectionBackground: "rgba(79,157,253,.35)",
  };
};

async function openSession(name) {
  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost";
  termsEl.append(host);

  const term = new Terminal({
    fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace',
    fontSize: 12.5,
    lineHeight: 1.2,
    cursorBlink: true,
    allowTransparency: true,
    scrollback: 5000,
    theme: theme(),
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(host);

  const s = { id, name, term, fit, host, dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  fit.fit();

  term.onData((d) => invoke("write_session", { id, data: d }).catch(() => {}));
  term.onResize(({ cols, rows }) => invoke("resize_session", { id, cols, rows }).catch(() => {}));

  try {
    s.unlisten.push(await listen(`pty:${id}`, (e) => term.write(e.payload)));
    s.unlisten.push(await listen(`pty-exit:${id}`, (e) => {
      s.dead = true;
      term.write(`\r\n\x1b[2m── ssh exited (${e.payload}) · ⌘W to close ──\x1b[0m\r\n`);
      renderTabs();
    }));
  } catch (err) {
    // `listen` is a core command and needs src-tauri/capabilities — without it the
    // session would open and then sit there mute.
    s.dead = true;
    term.write(`\x1b[31mcould not subscribe to the session: ${String(err)}\x1b[0m\r\n`);
    renderTabs();
    return;
  }

  try {
    const line = await invoke("open_session", { id, name, cols: term.cols, rows: term.rows });
    // Written locally, so seeing it proves the terminal renders even when the
    // remote end is slow or silent.
    term.write(`\x1b[2m${line}\x1b[0m\r\n`);
  } catch (err) {
    s.dead = true;
    term.write(`\r\n\x1b[31m${String(err)}\x1b[0m\r\n`);
  }
  renderTabs();
  term.focus();
}

function closeSession(id) {
  const s = sessions.get(id);
  if (!s) return;
  invoke("close_session", { id }).catch(() => {});
  s.unlisten.forEach((f) => f());
  s.term.dispose();
  s.host.remove();
  sessions.delete(id);
  if (activeId === id) activeId = [...sessions.keys()].pop() ?? null;
  showTab();
  renderTabs();
}

// activeId === null is the "All jacks" tab; anything else is a session.
function showTab() {
  browseEl.hidden = activeId !== null;
  termsEl.hidden = activeId === null;
  for (const s of sessions.values()) s.host.hidden = s.id !== activeId;
  if (activeId !== null) {
    const s = sessions.get(activeId);
    // The pane only has its real size once it's visible, so fit after the swap.
    requestAnimationFrame(() => { s.fit.fit(); s.term.focus(); });
  }
  renderDetail();
}

function renderTabs() {
  // The browse tab is the crumb — it names the selected folder and counts it.
  const label = group === null ? "All jacks" : group === "\0untagged" ? "Untagged" : group.split("/").join(" / ");
  const browse = `<div class="tab" data-id="" aria-selected="${activeId === null}">
      ${icon("layers")}<span class="lbl">${esc(label)}</span><span class="n">${shown.length}</span></div>`;
  tabsEl.innerHTML = browse + [...sessions.values()].map((s) => `
      <div class="tab ${s.dead ? "dead" : ""}" data-id="${s.id}" aria-selected="${s.id === activeId}">
        <span class="dot ${s.dead ? "down" : "up"}"></span>
        <span class="lbl">${esc(s.name)}</span>
        <span class="x" data-close="${s.id}">${icon("x")}</span>
      </div>`).join("");
}

tabsEl.addEventListener("click", (e) => {
  const close = e.target.closest("[data-close]")?.dataset.close;
  if (close) return closeSession(+close);
  const tab = e.target.closest("[data-id]");
  if (!tab) return;
  activeId = tab.dataset.id === "" ? null : +tab.dataset.id;
  showTab();
  renderTabs();
});

addEventListener("resize", () => {
  if (activeId !== null) sessions.get(activeId)?.fit.fit();
});

function cycleSession(d) {
  const ids = [null, ...sessions.keys()];
  if (ids.length < 2) return;
  const i = ids.indexOf(activeId);
  activeId = ids[(i + d + ids.length) % ids.length];
  showTab();
  renderTabs();
}

// ── context menu ───────────────────────────────────────────────────────────
const actions = new Map();   // id -> fn, rebuilt each time the menu opens

function showCtx(x, y, head, items) {
  actions.clear();
  ctxEl.innerHTML =
    (head ? `<div class="ctx-head">${esc(head)}</div><div class="ctx-sep"></div>` : "") +
    items.map((it, i) => {
      if (it === "-") return `<div class="ctx-sep"></div>`;
      actions.set(String(i), it.run);
      return `<div class="ctx-item ${it.danger ? "danger" : ""}" data-a="${i}">${icon(it.icon)}${esc(it.label)}</div>`;
    }).join("");
  ctxEl.hidden = false;
  // Flip back inside the window rather than overflowing it.
  const r = ctxEl.getBoundingClientRect();
  ctxEl.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  ctxEl.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
}
const hideCtx = () => { ctxEl.hidden = true; };

ctxEl.addEventListener("click", (e) => {
  const a = e.target.closest("[data-a]")?.dataset.a;
  hideCtx();
  actions.get(a)?.();
});

// The webview's own menu is Reload / Inspect Element — never useful here.
document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  const jackRow = e.target.closest("#list .jack");
  const groupRow = e.target.closest(".group");

  if (jackRow) {
    sel = +jackRow.dataset.i;
    render();
    const j = shown[sel];
    return showCtx(e.clientX, e.clientY, j.name, [
      { icon: "square-terminal", label: "Connect", run: () => connect(j.name) },
      { icon: "external-link", label: "Open in Terminal", run: () => connect(j.name, true) },
      { icon: "copy", label: "Copy ssh command", run: () => navigator.clipboard.writeText(j.command).catch(() => {}) },
      "-",
      { icon: "pencil", label: "Edit…", run: () => openJack(j) },
      { icon: "trash-2", label: "Delete", danger: true, run: () => removeJack(j.name) },
    ]);
  }

  if (groupRow) {
    const path = groupRow.dataset.path;
    if (!path || path === "\0untagged") return;   // All jacks / Untagged aren't real folders
    return showCtx(e.clientX, e.clientY, path, [
      { icon: "plus", label: "New device here…", run: () => openJack(null, path) },
      { icon: "folder-plus", label: "New subfolder…", run: () => newGroup(path) },
      "-",
      { icon: "pencil", label: "Rename…", run: () => renameGroup(path) },
      { icon: "trash-2", label: "Delete folder", danger: true, run: () => removeGroup(path) },
    ]);
  }

  showCtx(e.clientX, e.clientY, null, [
    { icon: "plus", label: "New device…", run: () => openJack(null, group) },
    { icon: "folder-plus", label: "New folder…", run: () => newGroup(null) },
    "-",
    { icon: "file-pen-line", label: "Open config file", run: () => invoke("open_config") },
  ]);
});
window.addEventListener("blur", hideCtx);
document.addEventListener("mousedown", (e) => { if (!e.target.closest("#ctx")) hideCtx(); });
window.addEventListener("resize", hideCtx);

// ── ask (one-line prompt) ──────────────────────────────────────────────────
let askResolve = null;
function ask(title, value = "", okLabel = "OK") {
  $("ask-title").textContent = title;
  askInput.value = value;
  askErr.hidden = true;
  $("ask-ok").textContent = okLabel;
  askWrap.hidden = false;
  askInput.focus();
  askInput.select();
  return new Promise((res) => (askResolve = res));
}
function closeAsk(v) { askWrap.hidden = true; askResolve?.(v); askResolve = null; }
askForm.addEventListener("submit", (e) => { e.preventDefault(); closeAsk(askInput.value.trim() || null); });
$("ask-cancel").addEventListener("click", () => closeAsk(null));
askWrap.addEventListener("mousedown", (e) => { if (e.target === askWrap) closeAsk(null); });

// ── jack sheet ─────────────────────────────────────────────────────────────
function openJack(j, prefillGroup) {
  editing = j?.name ?? null;
  $("sheet-title").textContent = j ? `Edit ${j.name}` : "New device";
  jfDelete.hidden = !j;
  jfDelete.innerHTML = `${icon("trash-2")}Delete`;
  jfErr.hidden = true;
  const f = jackForm.elements;
  f.name.value = j?.name ?? "";
  f.host.value = j?.host ?? "";
  f.user.value = j?.user ?? "";
  f.port.value = j?.port ?? "";
  f.key.value = j?.key ?? "";
  f.jump.value = j?.jump ?? "";
  f.os.value = j?.os ?? "";
  f.desc.value = j?.desc ?? "";
  f.tags.value = (j?.tags ?? (prefillGroup && prefillGroup !== "\0untagged" ? [prefillGroup] : [])).join(", ");
  f.forward.value = (j?.forward ?? []).join(", ");
  $("jacknames").innerHTML = all.map((x) => `<option value="${esc(x.name)}">`).join("");
  $("oschoices").innerHTML = OS_CHOICES.map((o) => `<option value="${o}">`).join("");
  sheetWrap.hidden = false;
  f.name.focus();
}
const closeJack = () => { sheetWrap.hidden = true; };
const list2 = (s) => s.split(",").map((x) => x.trim()).filter(Boolean);

jackForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = jackForm.elements;
  const port = f.port.value.trim();
  if (port && !/^\d+$/.test(port)) return showErr(jfErr, "port has to be a number");
  try {
    await invoke("save_jack", {
      original: editing,
      jack: {
        name: f.name.value.trim(),
        host: f.host.value.trim(),
        user: f.user.value.trim() || null,
        port: port ? +port : null,
        key: f.key.value.trim() || null,
        jump: f.jump.value.trim() || null,
        os: f.os.value.trim() || null,
        desc: f.desc.value.trim() || null,
        tags: list2(f.tags.value),
        forward: list2(f.forward.value),
      },
    });
    for (const t of list2(f.tags.value)) pending.delete(t);
    closeJack();
    await load();
  } catch (err) { showErr(jfErr, String(err)); }
});
$("jf-cancel").addEventListener("click", closeJack);
sheetWrap.addEventListener("mousedown", (e) => { if (e.target === sheetWrap) closeJack(); });
jfDelete.addEventListener("click", () => { const n = editing; closeJack(); removeJack(n); });

function showErr(el, msg) { el.textContent = msg; el.hidden = false; }

// ── mutations ──────────────────────────────────────────────────────────────
async function removeJack(name) {
  if ((await ask(`Delete "${name}"? This edits your config file.`, name, "Delete")) !== name) return;
  try { await invoke("delete_jack", { name }); sel = 0; await load(); }
  catch (e) { alertish(e); }
}

async function newGroup(parent) {
  const name = await ask(parent ? `New folder inside ${parent}` : "New folder", "", "Create");
  if (!name) return;
  const path = parent ? `${parent}/${name.replace(/^\/+|\/+$/g, "")}` : name.replace(/^\/+|\/+$/g, "");
  pending.add(path);
  expanded.add(path.split("/")[0]);
  group = path;
  render();
}

async function renameGroup(path) {
  const to = await ask(`Rename ${path} to`, path.split("/").pop(), "Rename");
  if (!to) return;
  const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  const next = parent ? `${parent}/${to}` : to;
  try {
    await invoke("rename_group", { from: path, to: next });
    if (pending.delete(path)) pending.add(next);
    if (group === path) group = next;
    await load();
  } catch (e) { alertish(e); }
}

async function removeGroup(path) {
  const n = all.filter((j) => j.tags.some((t) => t === path || t.startsWith(path + "/"))).length;
  const msg = `Remove folder "${path}" from ${n} device${n === 1 ? "" : "s"}? The devices stay.`;
  if ((await ask(msg, path, "Remove")) !== path) return;
  try {
    await invoke("delete_group", { path });
    pending.delete(path);
    if (group === path || group?.startsWith(path + "/")) group = null;
    await load();
  } catch (e) { alertish(e); }
}

function alertish(e) {
  const box = detailEl.querySelector(".mono");
  if (box) { box.textContent = String(e); box.classList.add("err"); }
}

// ── events ─────────────────────────────────────────────────────────────────
searchBtn.addEventListener("click", () => openPalette());
$("newjack").addEventListener("click", () => openJack(null, group));
$("newgroup").addEventListener("click", () => newGroup(null));
$("editcfg").addEventListener("click", () => invoke("open_config"));

treeEl.addEventListener("click", (e) => {
  const el = e.target.closest(".group");
  if (!el) return;
  const path = el.dataset.path || null;
  // Clicking the triangle folds; clicking the row selects.
  if (e.target.closest(".twist") && el.dataset.hasKids === "true") {
    expanded.has(path) ? expanded.delete(path) : expanded.add(path);
  } else {
    group = path;
    if (el.dataset.hasKids === "true") expanded.add(path);
    sel = 0;
  }
  render();
});

listEl.addEventListener("click", (e) => {
  const row = e.target.closest(".jack");
  if (!row) return;
  sel = +row.dataset.i;
  render();
});
listEl.addEventListener("dblclick", (e) => {
  const row = e.target.closest(".jack");
  if (row) connect(shown[+row.dataset.i].name);
});

detailEl.addEventListener("click", async (e) => {
  const act = e.target.closest("[data-act]")?.dataset.act;
  const live = activeId !== null ? sessions.get(activeId) : null;
  const j = live ? all.find((x) => x.name === live.name) : shown[sel];
  if (!j) return;
  if (act === "connect") connect(j.name);
  if (act === "disconnect") closeSession(activeId);
  if (act === "edit") openJack(j);
  if (act === "copy") {
    const btn = e.target.closest("[data-act]");
    try { await navigator.clipboard.writeText(j.command); btn.innerHTML = icon("check"); }
    catch { btn.innerHTML = icon("circle-off"); }
    setTimeout(() => (btn.innerHTML = icon("copy")), 900);
  }
});

presultsEl.addEventListener("click", (e) => {
  const row = e.target.closest("[data-pi]");
  if (!row) return;
  const j = palMatches()[+row.dataset.pi];
  closePalette();
  connect(j.name);
});
pq.addEventListener("input", () => { palSel = 0; renderPalette(); });
paletteEl.addEventListener("mousedown", (e) => { if (e.target === paletteEl) closePalette(); });

document.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;

  // A sheet is modal: let it have the keyboard, bar Escape.
  if (!sheetWrap.hidden || !askWrap.hidden) {
    if (e.key === "Escape") { e.preventDefault(); askWrap.hidden ? closeJack() : closeAsk(null); }
    return;
  }
  if (!ctxEl.hidden && e.key === "Escape") { e.preventDefault(); return hideCtx(); }

  if (mod && e.key === "k") { e.preventDefault(); return palOpen() ? closePalette() : openPalette(); }
  if (mod && e.key === "n") { e.preventDefault(); return openJack(null, group); }
  if (mod && e.key === "[") { e.preventDefault(); return cycleSession(-1); }
  if (mod && e.key === "]") { e.preventDefault(); return cycleSession(1); }

  // A live session owns the keyboard — every keystroke belongs to ssh, not to us.
  // Only the window-level shortcuts above and these get intercepted.
  if (activeId !== null) {
    if (mod && e.key === "w") { e.preventDefault(); closeSession(activeId); }
    return;
  }

  if (palOpen()) {
    const rows = palMatches();
    if (e.key === "Escape") { e.preventDefault(); closePalette(); }
    else if (e.key === "ArrowDown") { e.preventDefault(); palSel = (palSel + 1) % Math.max(1, rows.length); renderPalette(); }
    else if (e.key === "ArrowUp") { e.preventDefault(); palSel = (palSel - 1 + rows.length) % Math.max(1, rows.length); renderPalette(); }
    else if (e.key === "Enter" && rows[palSel]) { e.preventDefault(); const n = rows[palSel].name; closePalette(); connect(n); }
    return;
  }

  if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) { e.preventDefault(); move(1); }
  else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) { e.preventDefault(); move(-1); }
  else if (e.key === "Enter" && shown[sel]) { e.preventDefault(); connect(shown[sel].name); }
  else if (e.key === "Escape") { group = null; sel = 0; render(); }
  else if (shown[sel] && (e.key === "Backspace" || e.key === "Delete")) { e.preventDefault(); removeJack(shown[sel].name); }
  else if (mod && e.key === "e") { e.preventDefault(); invoke("open_config"); }
  else if (mod && e.key === "r") { e.preventDefault(); load(); }
  // Just start typing, like fzf — the palette opens carrying the keystroke.
  else if (!mod && !e.altKey && e.key.length === 1) { e.preventDefault(); openPalette(e.key); }
});

// ── load ───────────────────────────────────────────────────────────────────
async function load() {
  try {
    all = await invoke("jacks");
    // Open the first level once, on the first load only — doing it every time
    // would re-open folders the moment the window regains focus.
    if (!seeded) {
      for (const j of all) for (const t of j.tags) expanded.add(t.split("/")[0]);
      seeded = true;
    }
    render();
    refreshProbes();
  } catch (e) {
    treeEl.innerHTML = "";
    listEl.innerHTML = `<p class="empty">${esc(e)}</p>`;
  }
}

const PROBE_EVERY = 30_000;
async function refreshProbes() {
  // Throttled because load() runs on every window focus, and a sweep opens a
  // socket to every jack. Alt-tabbing shouldn't hammer the whole estate.
  if (Date.now() - lastProbe < PROBE_EVERY) return;
  lastProbe = Date.now();
  try {
    probes = new Map((await invoke("probe")).map((p) => [p.name, p]));
    render();
  } catch { /* a failed sweep just leaves the dots hollow */ }
}

load();
setInterval(refreshProbes, PROBE_EVERY);
// The config is a file you edit by hand, so pick up changes when the window comes back.
window.addEventListener("focus", load);
