// Classic script, no bundler - see the load order in ui/index.html.
// Terminal tabs. Each session is an xterm bound to a pty in the Rust side.

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
  // The 16 are `--a-*` in app.css, not literals here, so light and dark are one
  // block each in the file that owns every other colour. Left as xterm's own
  // defaults they are the raw VT hexes, which is a #00ff00 `ls` beside a palette
  // that was tuned - the loudest thing in the window and the only undesigned one.
  const ansi = {};
  for (const name of ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"]) {
    ansi[name] = v(`--a-${name}`, "");
    // xterm's key for bright red is `brightRed`, and the token is `--a-bright-red`.
    ansi[`bright${name[0].toUpperCase()}${name.slice(1)}`] = v(`--a-bright-${name}`, "");
  }
  return {
    background: "rgba(0,0,0,0)",
    foreground: v("--fg", "#f0f0f4"),
    cursor: v("--accent", "#4f9dfd"),
    // What is drawn *under* a block cursor: the app's own ground, or the character
    // it covers is painted in a colour the theme never chose.
    cursorAccent: v("--bg", "#17171a"),
    selectionBackground: "rgba(79,157,253,.35)",
    ...ansi,
  };
};

/// A live session keeps its pty and its scrollback - the theme and the size are only
/// how it is drawn. The refit is what tells the far end the shape changed.
function restyleTerminals() {
  for (const s of sessions.values()) {
    if (!s.term) continue;
    s.term.options.theme = theme();
    s.term.options.fontSize = termFont();
    s.fit?.fit();
  }
}

function makeTerm(host) {
  const term = new Terminal({
    fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace',
    fontSize: termFont(),
    // Not 1.2: a box-drawing glyph is exactly one cell tall, so any leading at all
    // breaks every vertical rule a TUI draws into a dashed line. The DOM renderer
    // takes these from the font and cannot stretch them, so the cell has to fit.
    lineHeight: 1,
    cursorBlink: true,
    // An unfocused tab is not a live one, and a solid block in both says otherwise.
    cursorInactiveStyle: "outline",
    allowTransparency: true,
    scrollback: 5000,
    theme: theme(),
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  const search = new SearchAddon.SearchAddon();
  term.loadAddon(search);
  // A link in output belongs to whatever wrote it, so it goes to the browser through
  // `open_link`, which takes http(s) and nothing else - never to a webview of ours.
  term.loadAddon(new WebLinksAddon.WebLinksAddon((_, uri) => {
    invoke("open_link", { url: uri }).catch(alertish);
  }));
  term.open(host);
  // No webgl renderer: it has to composite against the transparent background the
  // macOS vibrancy needs, and leaves the previous frame behind when it does.
  return { term, fit, search };
}

/// A tab is a view of one thing, so asking for that thing again is a request to look
/// at it, not to open a second one. `key` is what the tab is *of*: a one-shot check
/// is not the same tab as a shell on the same device, and a web tab carries its url,
/// because a device whose address has changed since is a different page. A dead tab
/// is not a view of anything - Enter in it dials again, and a fresh click should not
/// land you in a corpse.
function showOpen(key) {
  const open = [...sessions.values()].find((s) => s.key === key && !s.dead);
  if (!open) return false;
  activeId = open.id;
  showTab();
  renderTabs();
  renderTree();
  return true;
}

/// A broadcast group: several `openSession()`s that share a tab and, when `on`, share
/// their keystrokes. Kept as an object each session points at, so a pane closing
/// removes itself from `live` without a scan and the tab can render the count without
/// walking the sessions map.
///
/// The "loud indicator" is here: the tab chip and every pane border go accent when
/// `on`, and one click on the chip flips it. Off means the focused pane still sends
/// input (fanning would be surprising), but every pane still receives its own output.
let nextGid = 1;
function makeBroadcast(names) {
  return { gid: nextGid++, on: true, names, live: new Set() };
}
const inActive = (s) => {
  if (activeId === null) return false;
  const a = sessions.get(activeId);
  return s === a || (a?.bcast && s.bcast === a.bcast);
};

/// Open every marked device as its own session inside one broadcast tab. Marks that
/// aren't reachable over ssh are named in a pill and skipped: the grid is a grid of
/// terminals, and a webview or an RDP canvas mixed into it would be a bigger change
/// than this feature is. See `openSession` for how a single pane works.
async function openBroadcast(marks) {
  const ssh = marks.filter((m) => m.ssh);
  const skipped = marks.filter((m) => !m.ssh);
  if (ssh.length < 2) return alertish("A broadcast needs two or more ssh devices.");
  if (skipped.length) {
    // A note rather than an error - the marks that could be broadcast are being
    // broadcast, and the ones that couldn't are named so nothing goes silent.
    flash(`Broadcasting to ${ssh.length} · left out: ${skipped.map((m) => m.name).join(", ")}`);
  }
  const b = makeBroadcast(ssh.map((m) => m.name));
  // In parallel: each is a separate ssh dial, and awaiting them one at a time made a
  // twelve-device broadcast pop in one pane every render frame instead of together.
  await Promise.all(ssh.map((j) => openSession(j.name, null, b)));
  // Focus the first one - showTab lays the whole group out either way.
  const first = [...sessions.values()].find((s) => s.bcast === b);
  if (first) { activeId = first.id; showTab(); renderTabs(); }
}

/// `task` is "ping" or "trace": the same pty and the same tab, running a one-shot
/// check instead of a shell. It runs on the jump host when there is one, because a
/// device behind a bastion isn't reachable from here to begin with. `bcast`, if
/// given, joins this session to a broadcast group - see `openBroadcast`.
async function openSession(name, task = null, bcast = null) {
  // A broadcast pane is never a "there is already one of these" hit: opening a grid
  // of the same twelve devices twice is two grids, not one focus.
  const key = bcast ? `bcast:${bcast.gid}:${name}` : `term:${task ?? ""}:${name}`;
  if (!bcast && showOpen(key)) return;
  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost" + (bcast ? " bcasthost" : "");
  if (bcast) {
    host.dataset.name = name;
    host.addEventListener("mousedown", () => { activeId = id; showTab(); renderTabs(); }, true);
  }
  termsEl.append(host);

  const { term, fit, search } = makeTerm(host);

  const s = { id, name, task, kind: "term", key, term, fit, search, host, dead: false, unlisten: [], bcast };
  sessions.set(id, s);
  if (bcast) bcast.live.add(id);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();
  // One frame, so the pane has its real width before the pty is told a size. Sized
  // here and resized again a frame later, the shell gets a SIGWINCH mid-login and
  // anything drawing with cursor moves - a fastfetch box, any TUI - is left painting
  // on a grid that has since reflowed underneath it.
  await new Promise((r) => requestAnimationFrame(r));
  fit.fit();

  term.onData((d) => {
    // A dead tab keeps the keyboard and has nowhere to send it, so Enter dials the
    // same device again instead of making you close it and find it in the list.
    if (s.dead) {
      if (d === "\r") { dropTab(id); openSession(name, task, s.bcast); }
      return;
    }
    // Broadcast on: every live sibling gets the same keystroke. Off: only the focused
    // pane gets it, so a stray Ctrl+C after unfocusing a runaway box doesn't reach
    // the other eleven. A pane that died mid-broadcast is silently skipped rather
    // than resurrecting itself with the next keypress - it takes an Enter for that,
    // above.
    if (s.bcast?.on) {
      for (const other of s.bcast.live) {
        invoke("write_session", { id: other, data: d }).catch(() => {});
      }
      return;
    }
    invoke("write_session", { id, data: d }).catch(() => {});
  });
  term.onResize(({ cols, rows }) => invoke("resize_session", { id, cols, rows }).catch(() => {}));

  // Said before anything is spawned, so a slow or silent host still shows that the
  // terminal is alive. It is ours to take back: a login that draws with cursor moves -
  // fastfetch from a .zshrc - puts its box over whatever is already on screen, so the
  // first byte from the far end gets a clean one. Anything else we wrote - an error
  // on the way in - stays, because that is not ours to throw away.
  let ours = true;
  term.write(`\x1b[2m── ${task ? `${task} ` : ""}${name}… ──\x1b[0m\r\n`);

  try {
    s.unlisten.push(await listen(`pty:${id}`, (e) => {
      if (ours) {
        ours = false;
        term.write("\x1b[2J\x1b[3J\x1b[H");
      }
      term.write(e.payload);
    }));
    s.unlisten.push(await listen(`pty-exit:${id}`, (e) => {
      s.dead = true;
      s.bcast?.live.delete(id);
      term.write(`\r\n\x1b[2m── ${task ?? "ssh"} exited (${e.payload}) · ⏎ to ${task ? "run it again" : "reconnect"} · ${chord("w")} to close ──\x1b[0m\r\n`);
      renderTabs();
      renderTree();
    }));
  } catch (err) {
    // `listen` is a core command and needs src-tauri/capabilities - without it the
    // session would open and then sit there mute.
    s.dead = true;
    term.write(`\x1b[31mcould not subscribe to the session: ${String(err)}\x1b[0m\r\n`);
    renderTabs();
    return;
  }

  try {
    // The argv it returns is already on screen, in the detail pane's COMMAND box.
    await (task
      ? invoke("open_task", { id, name, task, cols: term.cols, rows: term.rows })
      : invoke("open_session", { id, name, cols: term.cols, rows: term.rows }));
  } catch (err) {
    ours = false;
    s.dead = true;
    term.write(`\r\n\x1b[31m${String(err)}\x1b[0m\r\n`);
  }
  renderTabs();
  term.focus();
}

function closeSession(id) {
  const s = sessions.get(id);
  if (!s) return;
  const closer = { rdp: "close_rdp_session", web: "close_web_view" }[s.kind] ?? "close_session";
  invoke(closer, { id }).catch(() => {});
  dropTab(id);
}

// Take the tab away without telling the far end anything - either it has already
// gone, or it never started.
function dropTab(id) {
  const s = sessions.get(id);
  if (!s) return;
  if (s.master != null) invoke("close_session", { id: s.master }).catch(() => {});
  clearInterval(s.wait);
  clearTimeout(s.noteTimer);
  s.unlisten.forEach((f) => f());
  s.term?.dispose();
  s.host.remove();
  sessions.delete(id);
  s.bcast?.live.delete(id);
  if (activeId === id) {
    // Focus stays inside the same broadcast group when a pane in it closes: a group
    // of twelve losing one pane should not throw you back to the "All jacks" tab.
    const sibling = s.bcast && [...sessions.values()].find((x) => x.bcast === s.bcast);
    activeId = sibling ? sibling.id : ([...sessions.keys()].pop() ?? null);
  }
  showTab();
  renderTabs();
  renderTree();
}

// activeId === null is the "All jacks" tab; anything else is a session.
function showTab() {
  // The bar searches one session's scrollback, so it does not follow you to the next.
  if (!findEl.hidden) closeFind();
  browseEl.hidden = activeId !== null;
  termsEl.hidden = activeId === null;
  const grid = sessions.get(activeId)?.bcast ?? null;
  termsEl.classList.toggle("grid", grid !== null);
  termsEl.classList.toggle("bcast-on", !!grid?.on);
  // Nearest-square grid, so 4 becomes 2×2 and 6 becomes 3×2 instead of auto-fit
  // filling one row and leaving one dangling below. Rows are 1fr; the terms
  // container's height is fixed by the pane layout above.
  if (grid) {
    const n = [...sessions.values()].filter((s) => s.bcast === grid).length;
    termsEl.style.setProperty("--cols", Math.max(1, Math.ceil(Math.sqrt(n))));
  } else {
    termsEl.style.removeProperty("--cols");
  }
  for (const s of sessions.values()) {
    // A broadcast group shows every one of its panes at once; a single tab shows
    // itself. `inActive` folds the two cases into one predicate.
    s.host.hidden = !inActive(s);
    s.host.classList.toggle("focused", grid !== null && s.id === activeId);
  }
  if (activeId !== null) {
    // Every visible pane needs its own fit after the grid template lands, or a
    // brand-new grid opens at whatever size the first pane thought it had.
    requestAnimationFrame(() => {
      if (grid) {
        for (const s of sessions.values()) if (inActive(s)) s.fit?.fit();
      }
      const s = sessions.get(activeId);
      s?.fit?.fit();
      (s?.term ?? s?.canvas)?.focus();
    });
  }
  placeWebViews();
  renderDetail();
}

// Where the visible web tab is, in page coordinates - or null when none is. A tooltip
// asks before drawing itself somewhere it would be invisible.
function webViewRect() {
  const s = sessions.get(activeId);
  if (!s || s.kind !== "web" || s.failed || modalOpen()) return null;
  const r = s.host.getBoundingClientRect();
  return r.width > 0 ? r : null;
}

// A child webview is an OS view stacked above the page: `hidden` does nothing to it
// and it covers every sheet. So each one is either exactly over its own host div or
// sized to nothing, and anything that opens on top has to call this again.
function placeWebViews() {
  for (const s of sessions.values()) {
    if (s.kind !== "web") continue;
    const r = s.host.getBoundingClientRect();
    // A failed tab shows our explanation instead, so the blank webview gets out of
    // the way rather than covering it.
    const shown = s.id === activeId && !modalOpen() && !s.failed && r.width > 0;
    invoke("place_web_view", {
      id: s.id,
      x: shown ? r.left : 0,
      y: shown ? r.top : 0,
      width: shown ? r.width : 0,
      height: shown ? r.height : 0,
    }).catch(() => {});
  }
}

// One observer instead of a `placeWebViews()` at every sheet's open and close: a new
// overlay that forgot the call would have a web tab painted straight over it, which is
// exactly the quiet failure the overlay rules in CLAUDE.md already warn about.
let overlayWatch = null;
function watchOverlays() {
  if (overlayWatch) return;
  overlayWatch = new MutationObserver(placeWebViews);
  for (const el of OVERLAYS()) {
    overlayWatch.observe(el, { attributes: true, attributeFilter: ["hidden"] });
  }
}

// Alongside the view, not in front of it. A page a webview won't load paints nothing
// and explains nothing - no certificate prompt, no error - so this is the only thing
// that can say why. Unlike Royal TS we can't offer the click-through: that prompt is
// WKNavigationDelegate's server-trust challenge, which wry owns and doesn't expose.
function checkWeb(s, url) {
  invoke("web_check", { url }).catch((e) => {
    if (!sessions.has(s.id) || s.failed) return;
    s.dead = true;
    s.failed = String(e);
    s.failedUrl = url;
    renderTabs();
    showWebFailure(s);
  });
}

// The webview is shrunk away and our own panel takes the pane: a blank page with no
// explanation is the thing that sends people hunting through the app for a bug.
function showWebFailure(s) {
  const cert = s.failed.includes("certificate");
  s.host.innerHTML = `<div class="panefail">
    <p class="why">${esc(s.failed)}</p>
    ${cert ? `<p class="fix">A device reached by its address has a certificate naming
      something else, and that never matches. <b>Trust it</b> hands the certificate to
      macOS the way the browser's "Always trust" does - macOS asks for your password,
      and the page opens here from then on.</p>` : ""}
    <div class="btns">
      ${cert ? `<button type="button" class="primary" data-web-cert="${esc(s.failedUrl)}">
        ${icon("check")}Trust it</button>` : ""}
      <button type="button" class="ghost" data-web-browser="${esc(s.name)}">
        ${icon("external-link")}Open in browser</button>
      ${/* Only removes our check - the webview still judges for itself, so on its own
            this can leave a blank page. Kept for when the certificate is already
            trusted and it is only rustls, which judges the name separately, refusing. */ ""}
      <button type="button" class="ghost" data-web-trust="${esc(s.failedUrl)}">
        ${icon("globe")}Skip the check</button>
    </div>
  </div>`;
  placeWebViews();
}

termsEl.addEventListener("click", async (e) => {
  const open = e.target.closest("[data-web-browser]");
  if (open) return invoke("open_url", { name: open.dataset.webBrowser }).catch(alertish);

  // Remembered, because our check is stricter than the webview: once the certificate
  // is trusted on this machine the page renders fine while rustls still refuses the
  // name, and being asked every time about a device you have already answered for is
  // the thing that makes people stop reading the message.
  const cert = e.target.closest("[data-web-cert]");
  const trust = e.target.closest("[data-web-trust]");
  const btn = cert ?? trust;
  if (!btn) return;
  const s = [...sessions.values()].find((x) => x.host.contains(btn));
  if (!s) return;

  try {
    if (cert) {
      // Show it before asking for it. Trusting a certificate you were never shown is
      // the thing Safari's dialog exists to prevent, and we fetch this one over a
      // connection we deliberately didn't verify - so it is exactly the moment where
      // someone in the way of that connection would get their certificate trusted.
      const c = await invoke("web_cert", { url: cert.dataset.webCert });
      const ok = await ask(
        `Trust this certificate?\n\n${c.subject}\nissued by ${c.issuer}\n` +
        `expires ${c.expires}\nSHA-256 ${c.fingerprint}`,
        null, "Trust it");
      if (!ok) return;
      // macOS raises its own authorisation prompt on top of this one.
      await invoke("web_trust_cert", { url: cert.dataset.webCert });
    } else {
      await invoke("web_trust", { url: trust.dataset.webTrust });
    }
  } catch (err) { return alertish(err); }

  // The page has to be loaded again to be judged again - the webview made its mind up
  // about that certificate before macOS changed its mind about it.
  const name = s.name;
  closeSession(s.id);
  openWebSession(name);
});

// ── files ──────────────────────────────────────────────────────────────────
// sftp in a tab. Ordinary HTML, unlike the web tab: nothing here is a foreign page,
// so it lives in our own webview and behaves like the rest of the app.
async function openFilesSession(name) {
  const key = `sftp:${name}`;
  if (showOpen(key)) return;
  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost filehost";
  termsEl.append(host);

  watchDrops();
  watchEdits();
  const s = { id, name, kind: "sftp", key, host, cwd: ".", dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();
  await listFiles(s);
}

async function listFiles(s, to = null) {
  if (to !== null) s.cwd = to;
  s.host.innerHTML = `<div class="files"><p class="loading">Listing ${esc(s.cwd)}…</p></div>`;
  try {
    // The server's own answer to where that took us, so the bar shows a real path
    // and the next hop starts from one.
    const at = await invoke("sftp_ls", { name: s.name, path: s.cwd });
    s.cwd = at.path;
    s.entries = at.entries;
    s.dead = false;
  } catch (e) {
    s.dead = true;
    s.entries = null;
    s.error = String(e);
    // Nothing here needs a shell - it needs a tty to answer a password in. So the tab
    // opens its own connection and asks in the pane, rather than sending you off to
    // open a terminal and come back.
    if (s.error.includes("Permission denied") && !s.master) return signIn(s);
  }
  renderTabs();
  renderFiles(s);
}

const fileSize = (n) => {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${i === 0 ? n : n.toFixed(1)} ${u[i]}`;
};

// A listing that came back with nothing looks exactly like a tab that failed to draw.
// And on macOS it is usually neither: TCC answers for these three with an empty
// directory rather than an error, which is a long afternoon if nobody says so.
function emptyNote(s) {
  const tcc = /^\/Users\/[^/]+\/(Desktop|Documents|Downloads)\/?$/.test(s.cwd);
  if (!tcc) return `<p class="fempty">Nothing here.</p>`;
  // The setting lives on the machine running sftp-server, so the button is only
  // honest when that machine is this one.
  const host = all.find((j) => j.name === s.name)?.host ?? "";
  const here = /^(localhost|127\.0\.0\.1|::1)$/i.test(host);
  return `<p class="fempty">Nothing here. macOS keeps Desktop, Documents and Downloads
    out of an ssh session's reach until <b>sftp-server</b> has Full Disk Access${
      here ? "" : ` on ${esc(s.name)}`}.</p>
    ${here ? `<div class="btns"><button type="button" class="ghost" data-files="fda">
      ${icon("settings")}Open Full Disk Access</button></div>` : ""}`;
}

function renderFiles(s) {
  if (!s.entries) {
    // The two answers to a listing that failed, rather than instructions to go and
    // find them: a shell is what authenticates the browse, and the retry is the click
    // you would otherwise make by closing the tab and opening it again.
    return void (s.host.innerHTML = `<div class="panefail">
      <p class="why">${esc(s.error)}</p>
      <p class="fix">A shell is what authenticates this - once one is open to
        ${esc(s.name)}, the listing appears here on its own.</p>
      <div class="btns">
        <button type="button" class="primary" data-files="term">
          ${icon("square-terminal")}Open a terminal</button>
        <button type="button" class="ghost" data-files="retry">
          ${icon("rotate-cw")}Try again</button>
      </div></div>`);
  }
  // Folders first, then names - the order every file browser has, so nobody has to
  // learn this one.
  const rows = [...s.entries].sort((a, b) =>
    a.dir === b.dir ? a.name.localeCompare(b.name) : (a.dir ? -1 : 1));

  s.host.innerHTML = `<div class="files">
    <div class="fpath">
      <button type="button" class="flat" data-up="1" data-tip="Up a folder">${icon("chevron-right")}</button>
      <span class="mono">${esc(s.cwd)}</span>
      <span class="fhint">${icon("download")}drop files here to upload</span>
      <button type="button" class="flat" data-files="mkdir" data-tip="New folder"
        data-tip-at="right">${icon("folder-plus")}</button>
    </div>
    <div class="flist">${rows.length ? "" : emptyNote(s)}${rows.map((e, i) => `
      <div class="frow" data-fi="${i}" data-dir="${e.dir}">
        ${icon(e.dir ? "folder" : "file-pen-line")}
        <span class="fname">${esc(e.name)}</span>
        <span class="fsize">${e.dir ? "" : esc(fileSize(e.size))}</span>
        <span class="fwhen">${esc(e.modified)}</span>
      </div>`).join("")}</div>
  </div>`;
  s.rows = rows;
}

termsEl.addEventListener("click", (e) => {
  const s = sessions.get(activeId);
  if (!s || s.kind !== "sftp") return;
  const act = e.target.closest("[data-files]")?.dataset.files;
  if (act === "term") return openSession(s.name);
  if (act === "fda") return invoke("open_full_disk_access").catch(alertish);
  if (act === "retry") return listFiles(s);
  if (act === "mkdir") return makeFolder(s);
  if (e.target.closest("[data-up]")) return upFolder(s);
  const row = e.target.closest("[data-fi]");
  if (!row || e.detail !== 2) return;
  const entry = s.rows[+row.dataset.fi];
  if (entry.dir) return listFiles(s, `${s.cwd}/${entry.name}`);
  downloadFile(s, entry.name);
});

// `ssh -N` on a pty, in the pane: a password, a host-key question or a passphrase is
// answered where it was asked. The connection it leaves behind is the one every sftp
// call in this tab rides, so it lives as long as the tab does.
async function signIn(s) {
  const id = nextId++;
  s.master = id;
  s.host.innerHTML = "";
  const { term, fit, search } = makeTerm(s.host);
  s.term = term;
  s.fit = fit;
  s.search = search;
  renderTabs();
  fit.fit();
  term.focus();
  term.onData((d) => invoke("write_session", { id, data: d }).catch(() => {}));

  try {
    s.unlisten.push(await listen(`pty:${id}`, (e) => term.write(e.payload)));
    s.unlisten.push(await listen(`pty-exit:${id}`, async () => {
      if (s.master !== id || s.entries) return;
      // A sign-in that worked ends this process too: ControlPersist backgrounds the
      // client the moment it has nothing left to do, leaving the connection behind. So
      // the socket, not the exit, is what says which of the two just happened.
      await new Promise((r) => setTimeout(r, 400));
      if (await invoke("sftp_ready", { name: s.name }).catch(() => false)) return connected(s);
      failedSignIn(s, "that sign-in didn't finish");
    }));
    term.write(`\x1b[2m── signing in to ${s.name} for files ──\x1b[0m\r\n`);
    await invoke("open_master", { id, name: s.name, cols: term.cols, rows: term.rows });
  } catch (e) {
    return failedSignIn(s, String(e));
  }
  watchConnection(s);
}

// The socket appears the moment ssh authenticates, so that is the signal - nothing to
// parse out of a terminal. Two minutes is long enough to find a password and short
// enough that a tab left open isn't polling all afternoon.
function watchConnection(s) {
  clearInterval(s.wait);
  const until = Date.now() + 120_000;
  s.wait = setInterval(async () => {
    if (!sessions.has(s.id) || Date.now() > until) return clearInterval(s.wait);
    if (await invoke("sftp_ready", { name: s.name }).catch(() => false)) connected(s);
  }, 800);
}

// The sign-in is over and the pane belongs to the file list now.
function connected(s) {
  clearInterval(s.wait);
  s.term?.dispose();
  s.term = s.fit = null;
  listFiles(s);
}

// Back to the panel, with the buttons, once signing in here didn't work - the shell
// someone opens by hand is still a connection this tab can ride.
function failedSignIn(s, why) {
  clearInterval(s.wait);
  s.term?.dispose();
  s.term = s.fit = null;
  s.master = null;
  s.dead = true;
  s.error = why;
  renderTabs();
  renderFiles(s);
  watchConnection(s);
}

// `..` rather than string surgery: the server knows where its own parent is.
const upFolder = (s) => listFiles(s, s.cwd === "." ? ".." : `${s.cwd}/..`);

// In the pane, not through alertish - that one paints the detail box red, which is
// the wrong colour for a file that arrived exactly as asked.
function fileNote(s, text, bad = false) {
  const bar = s.host.querySelector(".fpath");
  if (!bar) return;
  bar.querySelector(".fnote")?.remove();
  const note = document.createElement("span");
  note.className = `fnote ${bad ? "bad" : ""}`;
  note.textContent = text;
  bar.append(note);
  clearTimeout(s.noteTimer);
  s.noteTimer = setTimeout(() => note.remove(), 6000);
}

async function downloadFile(s, name, dir = false) {
  fileNote(s, `Fetching ${name}…`);
  try {
    const at = await invoke("sftp_get", { name: s.name, remote: `${s.cwd}/${name}`, recurse: dir });
    fileNote(s, `Saved to ${at}`);
  } catch (e) { fileNote(s, String(e), true); }
}

/// The three writes, all through one command. Every one of them re-lists rather than
/// patching the row: the far end is what decides whether it worked, and a listing is
/// one round trip on a connection that is already open.
async function fileEdit(s, op, path, to = "") {
  try { await invoke("sftp_edit", { name: s.name, op, path, to }); }
  catch (e) { return fileNote(s, String(e), true); }
  await listFiles(s);
}

async function makeFolder(s) {
  const name = await ask(`New folder in ${s.cwd}`, "", "Create");
  if (!name) return;
  fileEdit(s, "mkdir", `${s.cwd}/${name}`);
}

async function renameFile(s, entry) {
  const to = await ask(`Rename "${entry.name}" to`, entry.name, "Rename");
  if (!to || to === entry.name) return;
  fileEdit(s, "rename", `${s.cwd}/${entry.name}`, `${s.cwd}/${to}`);
}

async function removeFile(s, entry) {
  // A folder is the one that can't be undone by re-uploading, and sftp only removes an
  // empty one anyway - so it says which it is rather than asking the same question twice.
  const what = entry.dir ? `the folder "${entry.name}"` : `"${entry.name}"`;
  if (!(await ask(`Delete ${what} on ${s.name}?`, null, "Delete"))) return;
  fileEdit(s, entry.dir ? "rmdir" : "rm", `${s.cwd}/${entry.name}`);
}

/// Downloaded, handed to whatever this machine opens it with, and put back each time it
/// is saved - the watch is a thread in Rust, because nothing in the window survives the
/// tab being closed and an editor stays open longer than a file listing does.
async function editFile(s, name) {
  fileNote(s, `Opening ${name}…`);
  try {
    await invoke("sftp_open", { name: s.name, remote: `${s.cwd}/${name}` });
    fileNote(s, `${name} is open - saving it puts it back`);
  } catch (e) { fileNote(s, String(e), true); }
}

/// The saves come from a thread with no tab of its own, so the note has to find one -
/// and say so out loud when the tab has since been closed and something went wrong.
async function watchEdits() {
  if (watchEdits.on) return;
  watchEdits.on = true;
  try {
    await listen("sftp:saved", ({ payload: p }) => {
      const s = [...sessions.values()].find((x) => x.kind === "sftp" && x.name === p.name);
      if (!p.error) return void (s && fileNote(s, `${p.file} saved back to ${p.name}`));
      const why = `${p.file} could not be saved back to ${p.name}: ${p.error}`;
      s ? fileNote(s, why, true) : alertish(why);
    });
  } catch { /* no capability means no notice, not a broken tab */ }
}

// Real paths, straight from the webview's own drop event - a file picker would mean a
// plugin, and an <input type="file"> would give bytes to copy through JS rather than a
// path to hand sftp.
async function watchDrops() {
  if (watchDrops.on) return;
  watchDrops.on = true;
  try {
    await listen("tauri://drag-drop", async (e) => {
      const s = sessions.get(activeId);
      if (!s || s.kind !== "sftp") return;
      const paths = e.payload?.paths ?? [];
      if (!paths.length) return;
      fileNote(s, `Uploading ${paths.length} file${paths.length === 1 ? "" : "s"}…`);
      let failed = null;
      for (const local of paths) {
        try { await invoke("sftp_put", { name: s.name, local, remoteDir: s.cwd }); }
        catch (err) { failed = String(err); break; }
      }
      await listFiles(s);
      if (failed) fileNote(s, failed, true);
    });
  } catch { /* no capability means no drag-drop, not a broken tab */ }
}

async function openWebSession(name) {
  const key = `web:${name}:${all.find((j) => j.name === name)?.url ?? ""}`;
  if (showOpen(key)) return;
  watchOverlays();
  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost webhost";
  termsEl.append(host);

  const s = { id, name, kind: "web", key, host, dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();

  // The rect only exists once the host is laid out and visible.
  await new Promise((r) => requestAnimationFrame(r));
  const r = host.getBoundingClientRect();
  let url;
  try {
    url = await invoke("open_web_view", {
      id, name, x: r.left, y: r.top, width: r.width, height: r.height,
    });
  } catch (e) {
    // Nothing was ever shown in it - a dead tab here is one more thing to close for
    // a page that opened somewhere else, or never existed.
    dropTab(id);
    return alertish(e);
  }

  // Follow the page where it actually goes. A Synology's http port redirects to its
  // https one in JavaScript, so the certificate that blanks the tab belongs to a url
  // no http client of ours would ever have seen.
  try {
    s.unlisten.push(await listen(`web-nav:${id}`, (e) => {
      if (e.payload !== url && !s.failed) checkWeb(s, e.payload);
    }));
  } catch { /* no capability means no redirect notice, not a broken tab */ }

  checkWeb(s, url);
}

function renderTabs() {
  // The browse tab is the crumb - it names the selected folder and counts it.
  const label = group === null ? "All jacks" : groupLabel().split("/").join(" / ");
  const browse = `<div class="tab" data-id="" aria-selected="${activeId === null}">
      ${icon("layers")}<span class="lbl">${esc(label)}</span><span class="n">${shown.length}</span></div>`;
  // Broadcast groups collapse into one chip: N panes, one tab. The chip is aria-
  // selected whenever any of its panes has focus, and its own dot pulses when input
  // is fanning out. Non-broadcast sessions render as before.
  const seen = new Set();
  const chips = [];
  for (const s of sessions.values()) {
    if (s.bcast) {
      if (seen.has(s.bcast.gid)) continue;
      seen.add(s.bcast.gid);
      const b = s.bcast;
      const anyActive = [...sessions.values()].some((x) => x.bcast === b && x.id === activeId);
      const alive = b.live.size;
      chips.push(`<div class="tab bcast ${b.on ? "on" : ""}" data-gid="${b.gid}" aria-selected="${anyActive}">
        <span class="tabkind">${icon("radio-tower")}</span>
        <span class="lbl">Broadcast · ${alive} of ${b.names.length}</span>
        <button type="button" class="bmute" data-bmute="${b.gid}"
          data-tip="${b.on ? "Stop broadcasting keystrokes" : "Broadcast keystrokes to every pane"}"
          aria-pressed="${b.on}">${icon(b.on ? "radio-tower" : "square")}</button>
        <span class="x" data-bclose="${b.gid}" data-tip="Close all">${icon("x")}</span>
      </div>`);
      continue;
    }
    chips.push(`<div class="tab ${s.dead ? "dead" : ""}" data-id="${s.id}" aria-selected="${s.id === activeId}">
      <span class="dot ${s.dead ? "down" : "up"}"></span>
      ${s.kind === "rdp" ? `<span class="tabkind">${icon("monitor")}</span>` : ""}
      ${s.kind === "web" ? `<span class="tabkind">${icon("globe")}</span>` : ""}
      ${s.kind === "sftp" ? `<span class="tabkind">${icon("folder")}</span>` : ""}
      ${s.task ? `<span class="tabkind">${icon(s.task === "trace" ? "waypoints" : "plug")}</span>` : ""}
      <span class="lbl">${esc(s.task ? `${s.task} ${s.name}` : s.name)}</span>
      <span class="x" data-close="${s.id}" data-tip="Close  ${chord('w')}">${icon("x")}</span>
    </div>`);
  }
  tabsEl.innerHTML = browse + chips.join("");
  // Enough tabs and the strip scrolls even with every label squeezed, so the one you
  // just switched to has to be brought back into view. By hand, not scrollIntoView:
  // that one walks up to *any* scrollable ancestor, and it took the detail pane off
  // the side of the window with it.
  const active = tabsEl.querySelector('[aria-selected="true"]');
  if (active) {
    const right = active.offsetLeft + active.offsetWidth;
    if (active.offsetLeft < tabsEl.scrollLeft) tabsEl.scrollLeft = active.offsetLeft;
    else if (right > tabsEl.scrollLeft + tabsEl.clientWidth) {
      tabsEl.scrollLeft = right - tabsEl.clientWidth;
    }
  }
  markTabOverflow();
}

/// The strip hides its scrollbar, so a tab past the edge is a tab that simply isn't
/// there. Fade whichever edge still has something beyond it - and only that edge, or
/// a strip with three tabs in it looks like it is hiding some.
function markTabOverflow() {
  const room = tabsEl.scrollWidth - tabsEl.clientWidth;
  // A sub-pixel layout leaves a fraction of scrollable width on a strip that fits.
  tabsEl.classList.toggle("more-l", room > 1 && tabsEl.scrollLeft > 1);
  tabsEl.classList.toggle("more-r", room > 1 && tabsEl.scrollLeft < room - 1);
}
tabsEl.addEventListener("scroll", markTabOverflow, { passive: true });

tabsEl.addEventListener("click", (e) => {
  const close = e.target.closest("[data-close]")?.dataset.close;
  if (close) return closeSession(+close);
  const bclose = e.target.closest("[data-bclose]")?.dataset.bclose;
  if (bclose) return closeBroadcast(+bclose);
  const bmute = e.target.closest("[data-bmute]")?.dataset.bmute;
  if (bmute) return toggleBroadcast(+bmute);
  const bcast = e.target.closest("[data-gid]");
  if (bcast) {
    const gid = +bcast.dataset.gid;
    // Focusing a group tab lands on the first live pane, or on the first pane if
    // every one of them has died since.
    const first = [...sessions.values()].find((s) => s.bcast?.gid === gid && !s.dead)
      ?? [...sessions.values()].find((s) => s.bcast?.gid === gid);
    if (first) { activeId = first.id; showTab(); renderTabs(); }
    return;
  }
  const tab = e.target.closest("[data-id]");
  if (!tab) return;
  activeId = tab.dataset.id === "" ? null : +tab.dataset.id;
  showTab();
  renderTabs();
});

function toggleBroadcast(gid) {
  const s = [...sessions.values()].find((x) => x.bcast?.gid === gid);
  if (!s) return;
  s.bcast.on = !s.bcast.on;
  // Muting a *background* group must not repaint the grid you're actually looking
  // at - showTab reads the active group's own `.on`, whichever group that is.
  showTab();
  renderTabs();
}

function closeBroadcast(gid) {
  for (const s of [...sessions.values()]) if (s.bcast?.gid === gid) closeSession(s.id);
}

addEventListener("resize", () => {
  // A grid has as many terminals as it has cells, so one fit is not enough - every
  // visible pane recomputes its cols and rows against its share of the pane.
  for (const s of sessions.values()) if (inActive(s)) s.fit?.fit();
  placeWebViews();
  markTabOverflow();
});

function cycleSession(d) {
  // One step per chip, not per pane: a broadcast of twelve devices is one tab in the
  // strip, so ⌘] should skip past it rather than stepping through twelve panes.
  const stops = [null];
  const seen = new Set();
  for (const s of sessions.values()) {
    if (!s.bcast) { stops.push(s.id); continue; }
    if (seen.has(s.bcast.gid)) continue;
    seen.add(s.bcast.gid);
    stops.push(s.id);
  }
  if (stops.length < 2) return;
  const i = stops.indexOf(activeId);
  activeId = stops[(i + d + stops.length) % stops.length];
  showTab();
  renderTabs();
}

// ── find in a session ──────────────────────────────────────────────────────
// 5000 lines of scrollback and, until this, no way to look through them but the eye.
const findEl = $("find"), findQ = $("find-q"), findN = $("find-n");
// Which session the bar is searching. Not `activeId`: closing the bar has to clear the
// highlights off the terminal it was searching, and switching tabs is when it closes.
let findFor = null;

/// The colours the addon paints matches with. Read off the page rather than passed as
/// hexes: every other colour in the window comes from `:root`, and a match highlighted
/// in a colour the theme never chose is the one thing on screen that looks pasted on.
const findColors = () => {
  const css = getComputedStyle(document.body);
  const v = (n) => css.getPropertyValue(n).trim();
  return {
    decorations: {
      matchBackground: v("--row-hover"),
      matchBorder: v("--line"),
      matchOverviewRuler: v("--fg-faint"),
      activeMatchBackground: v("--accent"),
      activeMatchBorder: v("--accent"),
      activeMatchColorOverviewRuler: v("--accent"),
    },
  };
};

function findRun(back = false) {
  const s = sessions.get(findFor);
  const q = findQ.value;
  if (!s?.search || !q) { findN.textContent = ""; return; }
  const opts = { ...findColors(), incremental: !back };
  const hit = back ? s.search.findPrevious(q, opts) : s.search.findNext(q, opts);
  findN.textContent = hit ? "" : "no match";
}

/// Only where there is scrollback to search: a desktop, a page and a file list have
/// none, and a find box over them would be a control that does nothing.
function toggleFind() {
  const s = sessions.get(activeId);
  if (!s?.search) return;
  if (!findEl.hidden) return closeFind();
  findFor = activeId;
  // Painted here rather than in render(): the bar lives in the session pane, which
  // render() never touches, and this is the only moment it is about to be looked at.
  $("find-icon").innerHTML = icon("search");
  $("find-prev").innerHTML = icon("chevron-up");
  $("find-next").innerHTML = icon("chevron-down");
  $("find-x").innerHTML = icon("x");
  findEl.hidden = false;
  findQ.select();
  findQ.focus();
  findRun();
}

function closeFind() {
  findEl.hidden = true;
  findN.textContent = "";
  // The highlights belong to the search, so they go with it - and the keyboard goes
  // back to the shell it was taken from.
  const s = sessions.get(findFor);
  findFor = null;
  s?.search?.clearDecorations();
  if (s && s.id === activeId) s.term?.focus();
}

findQ.addEventListener("input", () => findRun());
findQ.addEventListener("keydown", (e) => {
  // The input has the keyboard here, so these never reach the window's own handler.
  if (e.key === "Escape") { e.preventDefault(); closeFind(); }
  else if (e.key === "Enter") { e.preventDefault(); findRun(e.shiftKey); }
});
$("find-prev").addEventListener("click", () => findRun(true));
$("find-next").addEventListener("click", () => findRun());
$("find-x").addEventListener("click", closeFind);

// ── remote desktop tabs ────────────────────────────────────────────────────
// Same tab strip as a terminal, but the pane is a <canvas> that Rust paints
// dirty rectangles onto. See src-tauri/src/rdp_session.rs for the other half.

// The browser names keys; RDP wants PC/AT set 1 scancodes. 0xE0__ marks the
// extended ones, which is exactly what Scancode::from_u16 looks for.
const SCANCODES = {
  Escape: 0x01, Digit1: 0x02, Digit2: 0x03, Digit3: 0x04, Digit4: 0x05, Digit5: 0x06,
  Digit6: 0x07, Digit7: 0x08, Digit8: 0x09, Digit9: 0x0a, Digit0: 0x0b, Minus: 0x0c,
  Equal: 0x0d, Backspace: 0x0e, Tab: 0x0f, KeyQ: 0x10, KeyW: 0x11, KeyE: 0x12,
  KeyR: 0x13, KeyT: 0x14, KeyY: 0x15, KeyU: 0x16, KeyI: 0x17, KeyO: 0x18, KeyP: 0x19,
  BracketLeft: 0x1a, BracketRight: 0x1b, Enter: 0x1c, ControlLeft: 0x1d, KeyA: 0x1e,
  KeyS: 0x1f, KeyD: 0x20, KeyF: 0x21, KeyG: 0x22, KeyH: 0x23, KeyJ: 0x24, KeyK: 0x25,
  KeyL: 0x26, Semicolon: 0x27, Quote: 0x28, Backquote: 0x29, ShiftLeft: 0x2a,
  Backslash: 0x2b, KeyZ: 0x2c, KeyX: 0x2d, KeyC: 0x2e, KeyV: 0x2f, KeyB: 0x30,
  KeyN: 0x31, KeyM: 0x32, Comma: 0x33, Period: 0x34, Slash: 0x35, ShiftRight: 0x36,
  NumpadMultiply: 0x37, AltLeft: 0x38, Space: 0x39, CapsLock: 0x3a,
  F1: 0x3b, F2: 0x3c, F3: 0x3d, F4: 0x3e, F5: 0x3f, F6: 0x40, F7: 0x41, F8: 0x42,
  F9: 0x43, F10: 0x44, NumLock: 0x45, ScrollLock: 0x46,
  Numpad7: 0x47, Numpad8: 0x48, Numpad9: 0x49, NumpadSubtract: 0x4a,
  Numpad4: 0x4b, Numpad5: 0x4c, Numpad6: 0x4d, NumpadAdd: 0x4e,
  Numpad1: 0x4f, Numpad2: 0x50, Numpad3: 0x51, Numpad0: 0x52, NumpadDecimal: 0x53,
  F11: 0x57, F12: 0x58,
  ControlRight: 0xe01d, AltRight: 0xe038, NumpadDivide: 0xe035, NumpadEnter: 0xe01c,
  Home: 0xe047, ArrowUp: 0xe048, PageUp: 0xe049, ArrowLeft: 0xe04b,
  ArrowRight: 0xe04d, End: 0xe04f, ArrowDown: 0xe050, PageDown: 0xe051,
  Insert: 0xe052, Delete: 0xe053, MetaLeft: 0xe05b, MetaRight: 0xe05c,
};

// Kept for the window's lifetime only, never written anywhere. Cleared on quit
// because it lives nowhere else.
const rdpCreds = new Map();

async function openRdpSession(name) {
  const j = all.find((x) => x.name === name);
  if (!j) return;
  // Before the credentials are asked for: a second desktop is a second login, and
  // being asked to sign in again for the session already on screen is the worst of it.
  const key = `rdp:${name}`;
  if (showOpen(key)) return;

  let creds = rdpCreds.get(name);
  if (!creds) {
    // The username is shown even when the config has one: a Windows box is usually
    // reached as a different account than ssh uses, and `DOMAIN\user` is not
    // something the file can guess.
    creds = await ask(`Sign in to ${j.host}`, "", "Connect", "password", j.user ?? "");
    if (!creds) return;
  }

  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost rdphost";
  const canvas = document.createElement("canvas");
  canvas.tabIndex = 0;   // so it can take the keyboard at all
  host.append(canvas);
  termsEl.append(host);
  const ctx = canvas.getContext("2d");

  const s = { id, name, kind: "rdp", key, canvas, host, dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();

  // Tiles start arriving before the invoke resolves, and setting canvas.width
  // *clears* the canvas - so anything painted before the size is known would be
  // wiped. Hold them until the server has told us how big the desktop is.
  let pending = [];
  const paint = (buf) => {
    const head = new DataView(buf, 0, 8);
    // 8-byte header (x, y, w, h as little-endian u16), then raw RGBA.
    const w = head.getUint16(4, true), h = head.getUint16(6, true);
    ctx.putImageData(
      new ImageData(new Uint8ClampedArray(buf, 8), w, h),
      head.getUint16(0, true),
      head.getUint16(2, true),
    );
  };
  const chan = new window.__TAURI__.core.Channel();
  chan.onmessage = (msg) => {
    const buf = msg instanceof ArrayBuffer ? msg : new Uint8Array(msg).buffer;
    pending ? pending.push(buf) : paint(buf);
  };

  try {
    const screen = await invoke("open_rdp_session", {
      id, name, user: creds.user, password: creds.password,
      width: 1280, height: 1024, onTile: chan,
    });
    // The server picks the size; asking for one is only a suggestion.
    canvas.width = screen.width;
    canvas.height = screen.height;
    const held = pending;
    pending = null;
    held.forEach(paint);
    rdpCreds.set(name, creds);
  } catch (err) {
    s.dead = true;
    // Rejected credentials must not be remembered, or the next attempt reuses them.
    rdpCreds.delete(name);
    renderTabs();
    return alertish(err);
  }

  // The pump thread ends when the far end hangs up; without this the picture just
  // freezes and the tab keeps showing a live dot.
  try {
    s.unlisten.push(await listen(`rdp-exit:${id}`, (e) => {
      s.dead = true;
      renderTabs();
      renderTree();
      if (e.payload) alertish(e.payload);
    }));
  } catch { /* no capability means no exit notice, not a broken session */ }

  const send = (kind, a = 0, b = 0, down = false) =>
    invoke("rdp_input", { id, kind, a, b, down }).catch(() => {});
  // The canvas is letterboxed to fit the pane, so pointer coordinates have to come
  // back through that scale before the far end sees them.
  const at = (e) => {
    const r = canvas.getBoundingClientRect();
    return [
      Math.round((e.clientX - r.left) * (canvas.width / r.width)),
      Math.round((e.clientY - r.top) * (canvas.height / r.height)),
    ];
  };
  canvas.addEventListener("mousemove", (e) => send("move", ...at(e)));
  canvas.addEventListener("mousedown", (e) => { canvas.focus(); send("button", e.button, 0, true); });
  canvas.addEventListener("mouseup", (e) => send("button", e.button, 0, false));
  canvas.addEventListener("contextmenu", (e) => e.stopPropagation());
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    send("wheel", e.deltaY > 0 ? -120 : 120);
  }, { passive: false });
  for (const [type, down] of [["keydown", true], ["keyup", false]]) {
    canvas.addEventListener(type, (e) => {
      const code = SCANCODES[e.code];
      if (code === undefined) return;
      // ⌘W and friends stay ours; everything else belongs to the remote desktop.
      if ((e.metaKey || e.ctrlKey) && ["w", "k", "n", "[", "]"].includes(e.key)) return;
      e.preventDefault();
      send("key", code, 0, down);
    });
  }

  renderTabs();
  canvas.focus();
}
