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
  if (!ctxEl.hidden && e.key === "Escape") { e.preventDefault(); return hideCtx(); }

  if (mod && e.key === "k") { e.preventDefault(); return palOpen() ? closePalette() : openPalette(); }
  if (mod && e.key === "n") { e.preventDefault(); return openJack(null, group); }
  if (mod && e.key === ",") { e.preventDefault(); return openSettings(); }
  if (mod && e.key === "[") { e.preventDefault(); return cycleSession(-1); }
  if (mod && e.key === "]") { e.preventDefault(); return cycleSession(1); }

  // A live session owns the keyboard - every keystroke belongs to ssh, not to us.
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
    else if (e.key === "Enter" && rows[palSel]) { e.preventDefault(); const n = rows[palSel].name; closePalette(); primary(n); }
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
async function load() {
  try {
    // Preferences first: the rest of the UI reads them.
    prefs = await invoke("settings").catch(() => ({}));
    if (!sshKeys.length) sshKeys = await invoke("ssh_keys").catch(() => []);
    colors = await invoke("colors").catch(() => ({}));
    cfgPath = await invoke("config_path").catch(() => "");
    spaces = await invoke("spaces").catch(() => []);
    tunnels = await invoke("tunnels").catch(() => []);
    all = await invoke("jacks");
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
    render();
    refreshProbes();
    syncTeam();
  } catch (e) {
    treeEl.innerHTML = "";
    listEl.innerHTML = `<p class="empty">${esc(e)}</p>`;
  }
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
