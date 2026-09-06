// Wires the keyboard and starts everything. Loaded last, so every function the
// handlers reach is already defined; see the load order in ui/index.html.

// ── events ─────────────────────────────────────────────────────────────────
searchBtn.addEventListener("click", () => openPalette());
$("viewmode").addEventListener("click", () => {
  listMode = listMode === "map" ? "list" : "map";
  render();
});
$("newjack").addEventListener("click", () => openJack(null, group));
$("newgroup").addEventListener("click", () => newGroup({ path: null }));
$("editcfg").addEventListener("click", () => invoke("open_config").catch(alertish));
// Called, not passed: the event would arrive as the pane name and match nothing.
$("settings").addEventListener("click", () => openSettings());

treeEl.addEventListener("click", (e) => {
  const el = e.target.closest(".group");
  if (!el) return;
  const id = el.dataset.group ? { path: el.dataset.path || null } : null;
  // Clicking the triangle folds; clicking the row selects.
  const foldable = el.dataset.hasKids === "true";
  if (e.target.closest(".twist") && foldable) {
    expanded.has(gkey(id)) ? expanded.delete(gkey(id)) : expanded.add(gkey(id));
    render();
  } else pickGroup(id, foldable);
});

listEl.addEventListener("click", (e) => {
  const act = e.target.closest("[data-first]")?.dataset.first;
  if (act === "new") return openJack(null, group);
  if (act === "import") return openSettings("import");
  if (act === "cfg") return invoke("open_config").catch(alertish);
  const row = e.target.closest(".jack");
  // Clicking under the rows clears the marks and keeps the selection.
  if (!row) {
    if (marked.size) {
      marked.clear();
      paintRows();
    }
    return;
  }
  // The map draws a jump host that isn't a device as a row with nothing behind it.
  if (row.dataset.i === undefined) return;
  const i = +row.dataset.i;
  if (picking(e)) return markToggle(i);
  if (e.shiftKey) return markRange(i);
  marked.clear();
  select(i);
  // e.detail is the click count, so this survives a re-render landing mid-gesture.
  if (e.detail === 2) primary(shown[sel].name);
});

// Drag a row onto a folder in the tree. Pointer events, not HTML5 drag and drop: the
// window's file-drop handler (uploads) takes that over on some platforms, and a web
// tab is an OS view above the page that would swallow a dragend anyway. A drag is a
// press that travels; a press that doesn't is still the click above.
let dragged = false; // the click that ends a drag is not a click
listEl.addEventListener("pointerdown", (e) => {
  const row = e.target.closest(".jack[data-i]");
  if (!row || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return;
  const j = shown[+row.dataset.i];
  if (!j) return;
  const js = marked.has(j.name) ? markedHere() : [j];
  const [x0, y0] = [e.clientX, e.clientY];
  let ghost = null;
  let over = null;
  const target = (ev) => {
    const el = document.elementFromPoint(ev.clientX, ev.clientY)?.closest("#tree .group");
    return el && (el.dataset.group || el.dataset.path === "") ? el : null;
  };
  const onMove = (ev) => {
    if (!ghost) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      listEl.setPointerCapture(e.pointerId);
      document.body.classList.add("dragging");
      ghost = document.createElement("div");
      ghost.className = "drag-ghost";
      ghost.textContent = js.length === 1 ? js[0].name : `${js.length} devices`;
      document.body.append(ghost);
    }
    ghost.style.transform = `translate(${ev.clientX + 12}px, ${ev.clientY + 12}px)`;
    const next = target(ev);
    if (next !== over) {
      over?.classList.remove("drop-on");
      next?.classList.add("drop-on");
      over = next;
    }
  };
  const end = () => {
    listEl.removeEventListener("pointermove", onMove);
    listEl.removeEventListener("pointerup", end);
    listEl.removeEventListener("lostpointercapture", end);
    if (!ghost) return;
    ghost.remove();
    document.body.classList.remove("dragging");
    over?.classList.remove("drop-on");
    // The click lands in this same turn; a release outside the window sends none.
    dragged = true;
    setTimeout(() => (dragged = false), 0);
    if (over) moveJacks(js, over.dataset.path || null);
  };
  listEl.addEventListener("pointermove", onMove);
  listEl.addEventListener("pointerup", end);
  listEl.addEventListener("lostpointercapture", end);
});
listEl.addEventListener(
  "click",
  (e) => {
    if (!dragged) return;
    dragged = false;
    e.stopImmediatePropagation();
  },
  true,
);

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
  if (act === "forward") {
    try {
      await invoke("open_forwards", { name: j.name });
    } catch (err) {
      alertish(err);
    }
    refreshTunnels();
  }
  if (act === "untunnel") {
    try {
      for (const t of tunnels.filter((x) => x.jack === j.name))
        await invoke("close_tunnel", { id: t.id });
    } catch (err) {
      alertish(err);
    }
    refreshTunnels();
  }
  if (act === "edit") openJack(j);
  if (act === "dup") openJack({ ...j, name: "" });
  if (act === "copy") {
    const btn = e.target.closest("[data-act]");
    try {
      await navigator.clipboard.writeText(j.command);
      btn.innerHTML = icon("check");
    } catch {
      btn.innerHTML = icon("circle-off");
    }
    setTimeout(() => (btn.innerHTML = icon("copy")), 900);
  }
});

presultsEl.addEventListener("click", (e) => {
  if (e.target.closest("[data-adhoc]")) {
    const q = adhoc();
    closePalette();
    return connect(q);
  }
  const row = e.target.closest("[data-pi]");
  if (!row) return;
  const j = palMatches()[+row.dataset.pi];
  closePalette();
  connect(j.name);
});
pq.addEventListener("input", () => {
  palSel = 0;
  renderPalette();
});
paletteEl.addEventListener("mousedown", (e) => {
  if (e.target === paletteEl) closePalette();
});

document.addEventListener("keydown", (e) => {
  const mod = chorded(e);
  const key = mod ? chordKey(e) : e.key;

  // A sheet is modal: it has the keyboard, bar Escape.
  if (sheetOpen()) {
    if (!askWrap.hidden && mod && askAgain === key) {
      e.preventDefault();
      return closeAsk(true);
    }
    if (e.key === "Escape") {
      e.preventDefault();
      if (!askWrap.hidden) closeAsk(null);
      else if (!setWrap.hidden) closeSettings();
      else if (!impWrap.hidden) closeImport();
      else leaveJack();
    }
    return;
  }
  // An open context menu is modal like a sheet.
  if (!ctxEl.hidden) {
    const rows = [...ctxEl.querySelectorAll(".ctx-item")];
    const at = rows.findIndex((r) => r.classList.contains("on"));
    if (e.key === "Escape") {
      e.preventDefault();
      hideCtx();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      const to = ((at < 0 ? (step > 0 ? -1 : 0) : at) + step + rows.length) % rows.length;
      rows.forEach((r, i) => r.classList.toggle("on", i === to));
    } else if (e.key === "Enter" && rows[at]) {
      e.preventDefault();
      rows[at].click();
    }
    return;
  }

  if (mod && key === "k") {
    e.preventDefault();
    return palOpen() ? closePalette() : openPalette();
  }
  if (mod && key === "n") {
    e.preventDefault();
    return openJack(null, group);
  }
  if (mod && key === ",") {
    e.preventDefault();
    return openSettings();
  }
  if (mod && key === "[") {
    e.preventDefault();
    return cycleSession(-1);
  }
  if (mod && key === "]") {
    e.preventDefault();
    return cycleSession(1);
  }
  if (mod && key === "f") {
    e.preventDefault();
    return toggleFind();
  }

  // Above the session guard on purpose: the palette is modal, so these keys were
  // never ssh's, and Escape must still close it while a tab is open.
  if (palOpen()) {
    const rows = palMatches();
    if (e.key === "Escape") {
      e.preventDefault();
      closePalette();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      palSel = (palSel + 1) % Math.max(1, rows.length);
      renderPalette();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      palSel = (palSel - 1 + rows.length) % Math.max(1, rows.length);
      renderPalette();
    } else if (e.key === "Enter" && rows[palSel]) {
      e.preventDefault();
      const n = rows[palSel].name;
      closePalette();
      primary(n);
    } else if (e.key === "Enter" && adhoc()) {
      e.preventDefault();
      const q = adhoc();
      closePalette();
      connect(q);
    }
    return;
  }

  // A live session owns the keyboard: only the window-level chords above and ⌘W
  // are intercepted; every other keystroke belongs to ssh.
  if (activeId !== null) {
    if (mod && key === "w") {
      e.preventDefault();
      closeSession(activeId);
    }
    return;
  }
  // A folder note is the one text box outside a sheet; its keystrokes are its own.
  if (e.target.closest("input, textarea")) {
    if (e.key === "Escape") e.target.blur();
    return;
  }

  if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
    e.preventDefault();
    move(1);
  } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
    e.preventDefault();
    move(-1);
  } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
    e.preventDefault();
    stepTree(e.key === "ArrowLeft" ? -1 : 1);
  } else if (e.key === "Enter" && shown[sel]) {
    e.preventDefault();
    primary(shown[sel].name);
  } else if (e.key === "Escape") {
    // Two-stage: marks first, folder and selection second.
    if (marked.size) {
      marked.clear();
      paintRows();
    } else {
      group = null;
      sel = 0;
      render();
    }
  } else if (shown[sel] && (e.key === "Backspace" || e.key === "Delete")) {
    e.preventDefault();
    const bulk = markedHere();
    if (bulk.length > 1) removeMarked(bulk);
    else removeJack(shown[sel].name);
  } else if (mod && key === "e") {
    e.preventDefault();
    invoke("open_config").catch(alertish);
  } else if (mod && key === "r") {
    e.preventDefault();
    load();
  } else if ((e.key === "?" && !mod) || (mod && key === "/")) {
    e.preventDefault();
    openSettings("keys");
  }
  // Just start typing: the palette opens carrying the keystroke.
  else if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1) {
    e.preventDefault();
    openPalette(e.key);
  }
});

// ── load ───────────────────────────────────────────────────────────────────
// #app waits hidden until the first load() so it arrives with its contents. The
// intro class comes off once played, or every re-render would re-stagger the rows.
function reveal() {
  if (!appEl.hidden) return;
  appEl.hidden = false;
  document.body.classList.add("intro");
  setTimeout(() => document.body.classList.remove("intro"), 700);
}

async function load() {
  try {
    // In parallel: load() runs on every focus and sits in front of the first paint.
    // Only `jacks` is left uncaught; its failure is what the error branch is for.
    let noteRows, tun, paths;
    [prefs, sshKeys, colors, paths, tun, noteRows, all] = await Promise.all([
      invoke("settings").catch(() => ({})),
      sshKeys.length ? sshKeys : invoke("ssh_keys").catch(() => []),
      invoke("colors").catch(() => ({})),
      invoke("config_path").catch(() => ({ list: "", own: "" })),
      invoke("tunnels").catch(() => ({ live: [], ended: [] })),
      invoke("notes").catch(() => []),
      invoke("jacks"),
    ]);
    ({ list: cfgPath, own: ownPath } = paths);
    takeTunnels(tun);
    notes = new Map(noteRows.map((n) => [n.path, n.note]));
    noteStamps = new Map(noteRows.map((n) => [n.path, n.stamp]));
    loadErr = "";
    // Open the first level on the first load only, or focus would re-open folders.
    if (!seeded) {
      for (const j of all) {
        for (const f of j.folders) expanded.add(gkey({ path: f.split("/")[0] }));
      }
      seeded = true;
    }
    applyTheme();
    applySidebar(prefs.sidebar);
    render();
    refreshProbes();
  } catch (e) {
    // A list already on screen stays: a share that is away is not an empty list.
    if (all.length) {
      if (String(e) !== loadErr) alertish(e);
      loadErr = String(e);
    } else {
      treeEl.innerHTML = "";
      listEl.innerHTML = `<p class="empty">${esc(e)}</p>`;
    }
  }
  reveal();
}

// Someone else's save shows up as the file's mtime or size moving. Different, not
// newer: a sync client keeps the source machine's mtime, and clocks disagree. Its own
// timer, because `refreshProbes` sits out when probing is off or the window is behind.
// `polling` holds the next tick back while one hangs on a share that has gone away.
let polling = false;
async function pollList() {
  if (polling) return;
  polling = true;
  try {
    const { stamp, conflict } = await invoke("list_stamp");
    if (listStamp !== null && stamp !== listStamp) await load();
    listStamp = stamp;
    if (conflict && conflict !== lastConflict) {
      lastConflict = conflict;
      flash(
        `"${conflict}" sits beside the list. A sync client makes one when two people save at once.`,
        true,
        {
          label: "Show",
          run: () => invoke("reveal_list").catch(alertish),
        },
      );
    }
  } catch {
    /* the next load() says why */
  } finally {
    polling = false;
  }
}

const PROBE_EVERY = 30_000;
// Throttled: load() runs on every focus and a sweep opens a socket to every jack.
// Skipped in the background too, except for the first sweep, so a window that opens
// behind something else still gets its dots.
async function refreshProbes() {
  if (prefs.probe === false) return;
  if (lastProbe && !document.hasFocus()) return;
  if (Date.now() - lastProbe < PROBE_EVERY) return;
  lastProbe = Date.now();
  try {
    probes = new Map((await invoke("probe")).map((p) => [p.name, p]));
    render();
  } catch {
    /* a failed sweep just leaves the dots hollow */
  }
}

// ── sidebar width ──────────────────────────────────────────────────────────
const SIDE_DEFAULT = 208;

// Clamped here as well as in Rust: the config can be hand-edited.
const applySidebar = (px) =>
  document.documentElement.style.setProperty(
    "--side-w",
    `${Math.round(Math.min(480, Math.max(150, px || SIDE_DEFAULT)))}px`,
  );

// Write the width once a drag ends, not sixty times a second.
function keepSidebar() {
  const px = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--side-w"));
  if (px !== prefs.sidebar) {
    prefs = { ...prefs, sidebar: px };
    invoke("save_settings", { next: prefs }).catch(alertish);
  }
  // xterm sizes itself to its host, which just changed.
  dispatchEvent(new Event("resize"));
}

const grip = $("sidegrip");
// Double-click puts the divider back.
grip.addEventListener("dblclick", () => {
  applySidebar(SIDE_DEFAULT);
  keepSidebar();
});
grip.addEventListener("pointerdown", (e) => {
  // Or the drag selects the text either side of it.
  e.preventDefault();
  grip.setPointerCapture(e.pointerId);
  grip.classList.add("on");
  const onMove = (ev) => applySidebar(ev.clientX);
  grip.addEventListener("pointermove", onMove);
  // Not pointerup: a drag crossing a web tab (an OS view above the page) loses the
  // pointer, and losing the capture is what happens either way.
  grip.addEventListener(
    "lostpointercapture",
    () => {
      grip.removeEventListener("pointermove", onMove);
      grip.classList.remove("on");
      keepSidebar();
    },
    { once: true },
  );
});

// ── update ─────────────────────────────────────────────────────────────────
// Once per launch, quiet unless there is something to install.
async function offerUpdate() {
  if (prefs.check_updates === false) return;
  // `undefined` is the endpoint failing, `null` is it answering "nothing new".
  const offer = await invoke("update_check").catch(() => undefined);
  if (offer !== undefined) checkedNow();
  if (offer) showUpdate(offer);
}

// The macOS menu's "Check for Updates…". The pill sits behind a sheet, so with
// settings open the answer goes beside its button instead.
listen("menu:check-update", () => checkUpdates(setWrap.hidden ? null : $("update-said"))).catch(
  () => {
    /* no capability, so the settings button is the only way in */
  },
);

// The macOS Help menu's "Keyboard Shortcuts".
listen("menu:shortcuts", () => openSettings("keys")).catch(() => {
  /* no capability, so ? is the only way in */
});

// A `patchbay://` link. Rust has already resolved it to a name in this config; the
// window opens it the way Enter would. See `takeLink` for one that arrived early.
listen("open:link", ({ payload: name }) => primary(name)).catch(() => {
  /* no capability, so a link only works on a cold start */
});

// The close button, held by Rust until the window answers: a live session is asked
// about. `quit` is `app.exit`, so tunnels are still closed on the way out.
listen("window:close", async () => {
  const live = liveSessions().length;
  if (live && !(await ask(`Quit with ${live} live session${live === 1 ? "" : "s"}?`, null, "Quit")))
    return;
  invoke("quit");
}).catch(() => {
  /* no capability - and then the close is held with nobody to answer it */
});

// Last time's tabs, in order, for devices that still exist. Ends back on the list.
async function restoreTabs() {
  const open = { term: openSession, web: openWebSession, sftp: openFilesSession };
  const back = lastTabs.filter((t) => open[t.kind] && all.some((j) => j.name === t.name));
  for (const t of back) await open[t.kind](t.name);
  if (back.length) {
    activeId = null;
    showTab();
    render();
  }
}

async function takeLink() {
  const name = await invoke("take_link").catch(() => null);
  if (name) primary(name);
}

// Offer to sweep what a previous install left in `~/.ssh` while the setting is off.
async function offerSshCleanup() {
  const left = await invoke("ssh_leftovers").catch(() => null);
  if (!left) return;
  showSshLeftover(left);
}

// Errands run behind the first paint.
load().then(restoreTabs).then(takeLink).then(offerUpdate).then(offerSshCleanup);
setInterval(refreshProbes, PROBE_EVERY);
setInterval(pollList, PROBE_EVERY);
// The config is hand-edited, so reload on focus.
window.addEventListener("focus", load);
// Not `matchMedia`: with a theme pinned, our own `color-scheme` fixes what
// `prefers-color-scheme` reports.
listen("tauri://theme-changed", () => {
  if (prefs.theme !== "light" && prefs.theme !== "dark") applyTheme();
}).catch(() => {
  /* no capability, so the theme only follows on reload */
});

// ── tooltips ───────────────────────────────────────────────────────────────
// One element at body level, so no overflow:hidden ancestor clips it.
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
  // Icon to padding is another `mouseover` for the same control, not a new tooltip.
  if (el === tipFor) return;
  clearTimeout(tipTimer);
  // Once one is up, the next follows the pointer straight away.
  const wait = tipEl.classList.contains("on") ? 0 : 150;
  tipFor = el;
  tipTimer = setTimeout(() => {
    tipEl.textContent = el.dataset.tip;
    const t = el.getBoundingClientRect();
    const r = tipEl.getBoundingClientRect();
    const gap = 7;
    // Above unless there is no room, never past a window edge. `tipAt` is for the few
    // controls hard against an edge.
    const below = t.top - r.height - gap < 4;
    const at = el.dataset.tipAt;
    let left =
      at === "left"
        ? t.left
        : at === "right"
          ? t.right - r.width
          : t.left + (t.width - r.width) / 2;
    left = Math.max(6, Math.min(left, innerWidth - r.width - 6));
    const top = below ? t.bottom + gap : t.top - r.height - gap;

    // A web tab is an OS view above the page: a tooltip landing on it is not drawn.
    // Flip to the other side if clear, otherwise skip it.
    const web = webViewRect();
    const hits = (y) =>
      web &&
      t.left < web.right &&
      t.left + r.width > web.left &&
      y < web.bottom &&
      y + r.height > web.top;
    let y = top;
    if (hits(y)) {
      const flipped = below ? t.top - r.height - gap : t.bottom + gap;
      if (hits(flipped) || flipped < 4) return hideTip();
      y = flipped;
    }
    // Placed before it is shown, or it paints one frame where the last one was.
    tipEl.style.left = `${Math.round(left)}px`;
    tipEl.style.top = `${Math.round(y)}px`;
    tipEl.classList.add("on");
  }, wait);
});
// Only when the pointer leaves the control: `mouseout` also fires from icon to padding.
document.addEventListener("mouseout", (e) => {
  const el = e.target.closest("[data-tip]");
  if (el && !el.contains(e.relatedTarget)) hideTip();
});
document.addEventListener("mousedown", hideTip);
addEventListener("blur", hideTip);
