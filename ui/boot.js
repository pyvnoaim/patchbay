// Classic script, no bundler — see the load order in ui/index.html.
// Loaded last: wires the keyboard and starts everything, so every function
// the handlers reach is already defined.

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
