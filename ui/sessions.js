// Classic script, no bundler — see the load order in ui/index.html.
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
  return {
    background: "rgba(0,0,0,0)",
    foreground: v("--fg", "#f0f0f4"),
    cursor: v("--accent", "#4f9dfd"),
    selectionBackground: "rgba(79,157,253,.35)",
  };
};

function makeTerm(host) {
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
  return { term, fit };
}

/// `task` is "ping" or "trace": the same pty and the same tab, running a one-shot
/// check instead of a shell. It runs on the jump host when there is one, because a
/// device behind a bastion isn't reachable from here to begin with.
async function openSession(name, task = null) {
  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost";
  termsEl.append(host);

  const { term, fit } = makeTerm(host);

  const s = { id, name, task, kind: "term", term, fit, host, dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();
  fit.fit();

  term.onData((d) => invoke("write_session", { id, data: d }).catch(() => {}));
  term.onResize(({ cols, rows }) => invoke("resize_session", { id, cols, rows }).catch(() => {}));

  // Said before anything is spawned, so a slow or silent host still shows that the
  // terminal is alive. It is ours to take back: a login that draws with cursor moves —
  // fastfetch from a .zshrc — puts its box over whatever is already on screen, so the
  // first byte from the far end gets a clean one. Anything else we wrote — an error
  // on the way in — stays, because that is not ours to throw away.
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
      term.write(`\r\n\x1b[2m── ${task ?? "ssh"} exited (${e.payload}) · ${chord("w")} to close ──\x1b[0m\r\n`);
      renderTabs();
      renderTree();
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

// Take the tab away without telling the far end anything — either it has already
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
  if (activeId === id) activeId = [...sessions.keys()].pop() ?? null;
  showTab();
  renderTabs();
  renderTree();
}

// activeId === null is the "All jacks" tab; anything else is a session.
function showTab() {
  browseEl.hidden = activeId !== null;
  termsEl.hidden = activeId === null;
  for (const s of sessions.values()) s.host.hidden = s.id !== activeId;
  if (activeId !== null) {
    const s = sessions.get(activeId);
    // The pane only has its real size once it's visible, so fit after the swap.
    // A canvas scales itself in CSS and just needs the keyboard.
    requestAnimationFrame(() => { s.fit?.fit(); (s.term ?? s.canvas)?.focus(); });
  }
  placeWebViews();
  renderDetail();
}

// Where the visible web tab is, in page coordinates — or null when none is. A tooltip
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
// and explains nothing — no certificate prompt, no error — so this is the only thing
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
      macOS the way the browser's "Always trust" does — macOS asks for your password,
      and the page opens here from then on.</p>` : ""}
    <div class="btns">
      ${cert ? `<button type="button" class="primary" data-web-cert="${esc(s.failedUrl)}">
        ${icon("check")}Trust it</button>` : ""}
      <button type="button" class="ghost" data-web-browser="${esc(s.name)}">
        ${icon("external-link")}Open in browser</button>
      ${/* Only removes our check — the webview still judges for itself, so on its own
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
      // connection we deliberately didn't verify — so it is exactly the moment where
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

  // The page has to be loaded again to be judged again — the webview made its mind up
  // about that certificate before macOS changed its mind about it.
  const name = s.name;
  closeSession(s.id);
  openWebSession(name);
});

// ── files ──────────────────────────────────────────────────────────────────
// sftp in a tab. Ordinary HTML, unlike the web tab: nothing here is a foreign page,
// so it lives in our own webview and behaves like the rest of the app.
async function openFilesSession(name) {
  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost filehost";
  termsEl.append(host);

  watchDrops();
  const s = { id, name, kind: "sftp", host, cwd: ".", dead: false, unlisten: [] };
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
    // Nothing here needs a shell — it needs a tty to answer a password in. So the tab
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
      <p class="fix">A shell is what authenticates this — once one is open to
        ${esc(s.name)}, the listing appears here on its own.</p>
      <div class="btns">
        <button type="button" class="primary" data-files="term">
          ${icon("square-terminal")}Open a terminal</button>
        <button type="button" class="ghost" data-files="retry">
          ${icon("rotate-cw")}Try again</button>
      </div></div>`);
  }
  // Folders first, then names — the order every file browser has, so nobody has to
  // learn this one.
  const rows = [...s.entries].sort((a, b) =>
    a.dir === b.dir ? a.name.localeCompare(b.name) : (a.dir ? -1 : 1));

  s.host.innerHTML = `<div class="files">
    <div class="fpath">
      <button type="button" class="flat" data-up="1" data-tip="Up a folder">${icon("chevron-right")}</button>
      <span class="mono">${esc(s.cwd)}</span>
      <span class="fhint">${icon("download")}drop files here to upload</span>
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
  const { term, fit } = makeTerm(s.host);
  s.term = term;
  s.fit = fit;
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

// The socket appears the moment ssh authenticates, so that is the signal — nothing to
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

// Back to the panel, with the buttons, once signing in here didn't work — the shell
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

// In the pane, not through alertish — that one paints the detail box red, which is
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

// Real paths, straight from the webview's own drop event — a file picker would mean a
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
  watchOverlays();
  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost webhost";
  termsEl.append(host);

  const s = { id, name, kind: "web", host, dead: false, unlisten: [] };
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
    // Nothing was ever shown in it — a dead tab here is one more thing to close for
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
  // The browse tab is the crumb — it names the selected folder and counts it.
  const label = group === null ? "All jacks" : groupLabel().split("/").join(" / ");
  const browse = `<div class="tab" data-id="" aria-selected="${activeId === null}">
      ${icon("layers")}<span class="lbl">${esc(label)}</span><span class="n">${shown.length}</span></div>`;
  tabsEl.innerHTML = browse + [...sessions.values()].map((s) => `
      <div class="tab ${s.dead ? "dead" : ""}" data-id="${s.id}" aria-selected="${s.id === activeId}">
        <span class="dot ${s.dead ? "down" : "up"}"></span>
        ${s.kind === "rdp" ? `<span class="tabkind">${icon("monitor")}</span>` : ""}
        ${s.kind === "web" ? `<span class="tabkind">${icon("globe")}</span>` : ""}
        ${s.kind === "sftp" ? `<span class="tabkind">${icon("folder")}</span>` : ""}
        ${s.task ? `<span class="tabkind">${icon(s.task === "trace" ? "waypoints" : "plug")}</span>` : ""}
        <span class="lbl">${esc(s.task ? `${s.task} ${s.name}` : s.name)}</span>
        <span class="x" data-close="${s.id}" data-tip="Close  ${chord('w')}">${icon("x")}</span>
      </div>`).join("");
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
  if (activeId !== null) sessions.get(activeId)?.fit?.fit();
  placeWebViews();
});

function cycleSession(d) {
  const ids = [null, ...sessions.keys()];
  if (ids.length < 2) return;
  const i = ids.indexOf(activeId);
  activeId = ids[(i + d + ids.length) % ids.length];
  showTab();
  renderTabs();
}

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

  const s = { id, name, kind: "rdp", canvas, host, dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();

  // Tiles start arriving before the invoke resolves, and setting canvas.width
  // *clears* the canvas — so anything painted before the size is known would be
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
