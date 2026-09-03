// Classic script, no bundler - see the load order in ui/index.html.
// Loaded last: wires the keyboard and starts everything, so every function
// the handlers reach is already defined.

// ── events ─────────────────────────────────────────────────────────────────
searchBtn.addEventListener("click", () => openPalette());
$("newjack").addEventListener("click", () => openJack(null, group));
$("newgroup").addEventListener("click", () => newGroup({ space: group?.space ?? null, path: null }));
$("newspace").addEventListener("click", () => newSpace());
$("editcfg").addEventListener("click", () => invoke("open_config"));
$("settings").addEventListener("click", openSettings);

treeEl.addEventListener("click", (e) => {
  const el = e.target.closest(".group");
  if (!el) return;
  // "All jacks" carries no group of its own; every other row does.
  const id = el.dataset.group ? { space: el.dataset.space || null, path: el.dataset.path || null } : null;
  // Clicking the triangle folds; clicking the row selects.
  const foldable = el.dataset.hasKids === "true";
  if (e.target.closest(".twist") && foldable) {
    expanded.has(gkey(id)) ? expanded.delete(gkey(id)) : expanded.add(gkey(id));
  } else {
    group = id;
    if (foldable) expanded.add(gkey(id));
    sel = 0;
    detailMode = "group";   // the pane describes the folder, not its first device
  }
  render();
});

listEl.addEventListener("click", (e) => {
  const act = e.target.closest("[data-first]")?.dataset.first;
  if (act === "new") return openJack(null, group);
  if (act === "import") return openImport();
  if (act === "cfg") return invoke("open_config");
  const row = e.target.closest(".jack");
  if (!row) return;
  select(+row.dataset.i);
  // e.detail is the click count, so this survives a re-render landing mid-gesture
  // in a way a separate dblclick listener does not.
  if (e.detail === 2) primary(shown[sel].name);
});

detailPane.addEventListener("click", async (e) => {
  const gact = e.target.closest("[data-gact]")?.dataset.gact;
  if (gact) {
    if (gact === "new") openJack(null, group);
    if (gact === "rename") renameGroup(group);
    if (gact === "del") removeGroup(group);
    // A space's own actions, on the space's own pane: a sync you asked for, and the
    // conflict answered where you are looking at it rather than two clicks away.
    if (gact === "sync") syncTeam();
    if (gact === "theirs" || gact === "mine") {
      teamCall(() => invoke("team_resolve", { space: group.space, keep: gact }));
    }
    return;
  }
  const act = e.target.closest("[data-act]")?.dataset.act;
  const live = activeId !== null ? sessions.get(activeId) : null;
  const j = live ? all.find((x) => x.name === live.name) : shown[sel];
  if (!j) return;
  if (act === "connect") connect(j.name);
  if (act === "ping" || act === "trace") openSession(j.name, act);
  if (act === "disconnect") closeSession(activeId);
  if (act === "web") openWeb(j.name);
  if (act === "rdp") openRdp(j.name);
  if (act === "vnc") openVnc(j.name);
  if (act === "files") openFilesSession(j.name);
  if (act === "forward") {
    try { await invoke("open_forwards", { name: j.name }); }
    catch (e) { alertish(e); }
    refreshTunnels();
  }
  if (act === "untunnel") {
    for (const t of tunnels.filter((x) => x.jack === j.name)) await invoke("close_tunnel", { id: t.id });
    refreshTunnels();
  }
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
  if (sheetOpen()) {
    if (e.key === "Escape") {
      e.preventDefault();
      if (!askWrap.hidden) closeAsk(null);
      else if (!setWrap.hidden) closeSettings();
      else if (!impWrap.hidden) closeImport();
      else closeJack();
    }
    return;
  }
  // An open menu is modal like a sheet: without this, typing behind it opens the
  // palette on top of the menu you were still reading.
  if (!ctxEl.hidden) {
    const rows = [...ctxEl.querySelectorAll(".ctx-item")];
    const at = rows.findIndex((r) => r.classList.contains("on"));
    if (e.key === "Escape") { e.preventDefault(); hideCtx(); }
    else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      const to = ((at < 0 ? (step > 0 ? -1 : 0) : at) + step + rows.length) % rows.length;
      rows.forEach((r, i) => r.classList.toggle("on", i === to));
    } else if (e.key === "Enter" && rows[at]) { e.preventDefault(); rows[at].click(); }
    return;
  }

  if (mod && e.key === "k") { e.preventDefault(); return palOpen() ? closePalette() : openPalette(); }
  if (mod && e.key === "n") { e.preventDefault(); return openJack(null, group); }
  if (mod && e.key === ",") { e.preventDefault(); return openSettings(); }
  if (mod && e.key === "[") { e.preventDefault(); return cycleSession(-1); }
  if (mod && e.key === "]") { e.preventDefault(); return cycleSession(1); }

  // Above the session guard on purpose: the palette is modal and holds the focus,
  // so those keys were never ssh's to begin with. Below it, Escape could not close
  // the palette at all while a tab was open.
  if (palOpen()) {
    const rows = palMatches();
    if (e.key === "Escape") { e.preventDefault(); closePalette(); }
    else if (e.key === "ArrowDown") { e.preventDefault(); palSel = (palSel + 1) % Math.max(1, rows.length); renderPalette(); }
    else if (e.key === "ArrowUp") { e.preventDefault(); palSel = (palSel - 1 + rows.length) % Math.max(1, rows.length); renderPalette(); }
    else if (e.key === "Enter" && rows[palSel]) { e.preventDefault(); const n = rows[palSel].name; closePalette(); primary(n); }
    return;
  }

  // A live session owns the keyboard - every keystroke belongs to ssh, not to us.
  // Only the window-level shortcuts above and these get intercepted.
  if (activeId !== null) {
    if (mod && e.key === "w") { e.preventDefault(); closeSession(activeId); }
    return;
  }

  if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) { e.preventDefault(); move(1); }
  else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) { e.preventDefault(); move(-1); }
  else if (e.key === "Enter" && shown[sel]) { e.preventDefault(); primary(shown[sel].name); }
  else if (e.key === "Escape") { group = null; sel = 0; render(); }
  else if (shown[sel] && (e.key === "Backspace" || e.key === "Delete")) { e.preventDefault(); removeJack(shown[sel].name, shown[sel].space ?? null); }
  else if (mod && e.key === "e") { e.preventDefault(); invoke("open_config"); }
  else if (mod && e.key === "r") { e.preventDefault(); load(); }
  // Just start typing, like fzf - the palette opens carrying the keystroke.
  else if (!mod && !e.altKey && e.key.length === 1) { e.preventDefault(); openPalette(e.key); }
});

// ── load ───────────────────────────────────────────────────────────────────
/// The window opens on an empty grid while the first load() fetches, so #app waits
/// hidden and arrives with its contents already in it. The class comes back off
/// once it has played: the lists are innerHTML and render() runs on every window
/// focus, so a rule left on the body would re-stagger every row on every alt-tab.
function reveal() {
  if (!appEl.hidden) return;
  appEl.hidden = false;
  document.body.classList.add("intro");
  setTimeout(() => document.body.classList.remove("intro"), 700);
}

async function load() {
  try {
    // Eight independent reads of the same config. Serially they were eight round
    // trips stacked in front of the first paint, and load() runs on every focus.
    // `jacks` is the only one left uncaught - it failing is what the error branch
    // below is for, and Promise.all rejecting is how it still gets there.
    [prefs, sshKeys, colors, cfgPath, spaces, spaceFiles, tunnels, all] = await Promise.all([
      invoke("settings").catch(() => ({})),
      sshKeys.length ? sshKeys : invoke("ssh_keys").catch(() => []),
      invoke("colors").catch(() => ({})),
      invoke("config_path").catch(() => ""),
      invoke("spaces").catch(() => []),
      invoke("space_files").catch(() => []),
      invoke("tunnels").catch(() => []),
      invoke("jacks"),
    ]);
    // Open the first level once, on the first load only - doing it every time
    // would re-open folders the moment the window regains focus.
    if (!seeded) {
      for (const j of all) {
        const space = j.space ?? null;
        expanded.add(gkey({ space, path: null }));
        for (const f of j.folders) expanded.add(gkey({ space, path: f.split("/")[0] }));
      }
      seeded = true;
    }
    applyTheme();
    applySidebar(prefs.sidebar);
    render();
    refreshProbes();
    syncTeam();
  } catch (e) {
    treeEl.innerHTML = "";
    listEl.innerHTML = `<p class="empty">${esc(e)}</p>`;
  }
  reveal();
}

/// Push what we changed, take what they changed. Deliberately not awaited by load():
/// a team server that has gone away must not hold the list up for a timeout, and with
/// no team configured this returns without touching the network at all.
async function syncTeam() {
  teams = await invoke("team_sync").catch(() => []);
  renderTeam();
  // A pull rewrote a space under whatever just read it. One reload, and the next
  // sync says nothing changed, so this can't loop.
  if (teams.some((t) => t.changed)) return load();
  render();
}

const PROBE_EVERY = 30_000;
async function refreshProbes() {
  // Throttled because load() runs on every window focus, and a sweep opens a
  // socket to every jack. Alt-tabbing shouldn't hammer the whole estate.
  if (prefs.probe === false) return;
  // And nothing at all while the window is in the background: the interval below
  // outlives your attention, and a socket to every host every 30s is a cost the
  // machine pays for a pane no one is reading. Coming back calls load(), which
  // calls this - so the dots are current the moment they're looked at again.
  // The first sweep goes ahead either way: a window that opens behind something
  // else, or on the other screen, would otherwise show no dots at all until clicked.
  if (lastProbe && !document.hasFocus()) return;
  if (Date.now() - lastProbe < PROBE_EVERY) return;
  lastProbe = Date.now();
  try {
    probes = new Map((await invoke("probe")).map((p) => [p.name, p]));
    render();
  } catch { /* a failed sweep just leaves the dots hollow */ }
}

// ── sidebar width ──────────────────────────────────────────────────────────
const SIDE_DEFAULT = 208;

/// Clamped here as well as in Rust: this is the one that stops you dragging your own
/// list off the screen, and the config could always have been edited by hand.
const applySidebar = (px) =>
  document.documentElement.style.setProperty(
    "--side-w", `${Math.round(Math.min(480, Math.max(150, px || SIDE_DEFAULT)))}px`);

/// The width as it now stands, written once. Both ways of changing it end here.
function keepSidebar() {
  const px = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--side-w"));
  // The config is a file people hand-edit, not somewhere to put sixty writes a second.
  if (px !== prefs.sidebar) {
    prefs = { ...prefs, sidebar: px };
    invoke("save_settings", { next: prefs }).catch(alertish);
  }
  // The panes either side just changed size, and xterm sizes itself to its host.
  dispatchEvent(new Event("resize"));
}

const grip = $("sidegrip");
// The divider convention everywhere else: double-click puts it back.
grip.addEventListener("dblclick", () => { applySidebar(SIDE_DEFAULT); keepSidebar(); });
grip.addEventListener("pointerdown", (e) => {
  // Or the pointer picks up the text either side of it on the way past.
  e.preventDefault();
  grip.setPointerCapture(e.pointerId);
  grip.classList.add("on");
  const move = (ev) => applySidebar(ev.clientX);
  grip.addEventListener("pointermove", move);
  // Not pointerup: a drag that crosses a web tab is a drag over an OS view above the
  // page, which takes the pointer with it. Losing the capture is the one thing that
  // happens either way, so it is what finishes the drag.
  grip.addEventListener("lostpointercapture", () => {
    grip.removeEventListener("pointermove", move);
    grip.classList.remove("on");
    keepSidebar();
  }, { once: true });
});

// ── update ─────────────────────────────────────────────────────────────────
/// Once per launch, and never in the way - see the pill in edit.js. An endpoint that
/// can't be reached and an app that is already current both say nothing: the check
/// runs on every launch, so the next one can raise it.
async function offerUpdate() {
  if (prefs.check_updates === false) return;
  // `undefined` is the endpoint failing, `null` is it answering "nothing new" - the
  // second is a check that happened and the settings pane says so.
  const offer = await invoke("update_check").catch(() => undefined);
  if (offer !== undefined) checkedNow();
  if (offer) showUpdate(offer);
}

// The macOS menu's "Check for Updates…" - it only says it was asked for; what a
// check looks like belongs to the window.
// The menu bar still works with a sheet open, and the pill is *behind* a sheet - so
// when settings is up, the answer goes to the line beside its button instead.
listen("menu:check-update", () => checkUpdates(setWrap.hidden ? null : $("update-said")))
  .catch(() => { /* no capability, so the settings button is the only way in */ });

// Behind the first paint: the window is for the device list, not for an errand.
load().then(offerUpdate);
setInterval(refreshProbes, PROBE_EVERY);
// The config is a file you edit by hand, so pick up changes when the window comes back.
window.addEventListener("focus", load);
// Not `matchMedia`: with a theme pinned, the page's own `color-scheme` fixes what
// `prefers-color-scheme` reports, so the machine changing its mind fires nothing.
listen("tauri://theme-changed", () => {
  if (prefs.theme !== "light" && prefs.theme !== "dark") applyTheme();
}).catch(() => { /* no capability, so the theme only follows on reload */ });

// ── tooltips ───────────────────────────────────────────────────────────────
// One element at body level so it escapes every overflow:hidden ancestor -
// a sheet clips a ::after tooltip, which is how this started.
const tipEl = $("tip");
let tipTimer = null;
let tipFor = null;

function hideTip() {
  clearTimeout(tipTimer);
  tipFor = null;
  tipEl.classList.remove("on");
}

document.addEventListener("mouseover", (e) => {
  const el = e.target.closest("[data-tip]");
  if (!el || !el.dataset.tip) return hideTip();
  // Crossing from the icon to the button's own padding is another `mouseover` for
  // the same control, not a new tooltip.
  if (el === tipFor) return;
  clearTimeout(tipTimer);
  // Once one is up, the next follows the pointer straight away: a wait between two
  // adjacent buttons reads as a flicker rather than as patience.
  const wait = tipEl.classList.contains("on") ? 0 : 150;
  tipFor = el;
  tipTimer = setTimeout(() => {
    tipEl.textContent = el.dataset.tip;
    const t = el.getBoundingClientRect();
    const r = tipEl.getBoundingClientRect();
    const gap = 7;
    // Above unless there is no room, and never past a window edge. `tipAt` is only for
    // the few that sit hard against an edge and would otherwise be clamped anyway -
    // centred is the default, and a button with room to centre should use it.
    const below = t.top - r.height - gap < 4;
    const at = el.dataset.tipAt;
    let left = at === "left" ? t.left : at === "right" ? t.right - r.width : t.left + (t.width - r.width) / 2;
    left = Math.max(6, Math.min(left, innerWidth - r.width - 6));
    const top = below ? t.bottom + gap : t.top - r.height - gap;

    // A web tab is an OS-level view above the page, so a tooltip landing on it is
    // simply not drawn - an invisible element that still thinks it's showing. Flip to
    // the other side if that side is clear, and otherwise don't pretend: every tooltip
    // that can land there labels a control you can already see.
    const web = webViewRect();
    const hits = (y) => web && t.left < web.right && t.left + r.width > web.left
      && y < web.bottom && y + r.height > web.top;
    let y = top;
    if (hits(y)) {
      const flipped = below ? t.top - r.height - gap : t.bottom + gap;
      if (hits(flipped) || flipped < 4) return hideTip();
      y = flipped;
    }
    // Placed before it is shown: made visible first, it paints one frame wherever the
    // last tooltip was.
    tipEl.style.left = `${Math.round(left)}px`;
    tipEl.style.top = `${Math.round(y)}px`;
    tipEl.classList.add("on");
    // Long enough not to flash at everything the pointer crosses on the way somewhere,
    // short enough that stopping on a button feels answered rather than waited on.
  }, wait);
});
// Only when the pointer actually leaves the control - `mouseout` also fires on the way
// from a button's icon to its padding, and hiding there is the flicker.
document.addEventListener("mouseout", (e) => {
  const el = e.target.closest("[data-tip]");
  if (el && !el.contains(e.relatedTarget)) hideTip();
});
document.addEventListener("mousedown", hideTip);
addEventListener("blur", hideTip);
