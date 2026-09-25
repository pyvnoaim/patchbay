// Session tabs: terminals, file listings, web views and remote desktops, all stacked in
// #terms with only the active one shown. Classic script sharing one global scope with
// the other ui/ files, so anything reaching across files stays inside a function body.

const { listen } = window.__TAURI__.event;
const tabsEl = $("tabs"),
  termsEl = $("terms"),
  browseEl = $("browse");
const sessions = new Map(); // id -> { id, name, term, fit, host, dead, unlisten[] }
let activeId = null;
let nextId = 1;

// xterm's theme from the `--a-*` tokens in app.css, so light and dark stay in one file.
const theme = () => {
  const css = getComputedStyle(document.body);
  const v = (name) => css.getPropertyValue(name).trim();
  const ansi = {};
  for (const name of ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"]) {
    ansi[name] = v(`--a-${name}`);
    ansi[`bright${name[0].toUpperCase()}${name.slice(1)}`] = v(`--a-bright-${name}`);
  }
  return {
    // The pane behind it is the app's own surface, so the terminal paints no ground.
    background: "rgba(0,0,0,0)",
    foreground: v("--fg"),
    cursor: v("--accent"),
    cursorAccent: v("--bg"),
    selectionBackground: v("--term-select"),
    ...ansi,
  };
};

// Re-theme and refit every live terminal; the refit tells the far end the shape changed.
function restyleTerminals() {
  for (const s of sessions.values()) {
    if (!s.term) continue;
    s.term.options.theme = theme();
    s.term.options.fontSize = termFont();
    s.fit?.fit();
  }
}

// What a Mac terminal sends that xterm doesn't: nothing at all for ⌘ with an arrow or
// ⌫, and ⌥-arrows as sequences no shell binds. These are readline's own keys, so bash,
// zsh and fish all understand them. Only on the shell's screen: in vim ^A is increment.
const MAC_KEYS = {
  "Meta+ArrowLeft": "\x01",
  "Meta+ArrowRight": "\x05",
  "Meta+Backspace": "\x15",
  "Alt+ArrowLeft": "\x1bb",
  "Alt+ArrowRight": "\x1bf",
};

// The terminal's own chords, or null for a key that belongs to the shell.
function termKey(e, term) {
  if (isMac && !e.ctrlKey && !e.shiftKey && e.metaKey !== e.altKey) {
    const send = MAC_KEYS[`${e.metaKey ? "Meta" : "Alt"}+${e.key}`];
    if (send && term.buffer.active.type === "normal") return () => term.input(send);
  }
  if (!chorded(e)) return null;
  // Read off `key` with Shift's answer beside it: Ctrl+Shift+= says "+".
  if (e.key === "+" || e.key === "=") return () => zoomTerminals(1);
  if (e.key === "-" || e.key === "_") return () => zoomTerminals(-1);
  if (e.code === "Digit0") return () => zoomTerminals(0);
  const key = chordKey(e);
  // Off macOS only: there ⌘A is the Edit menu's and arrives as a selectstart (makeTerm).
  if (key === "a") return () => selectWritten(term);
  // ⌘C and ⌘V are the Edit menu's; off macOS there is none, and xterm would send ^C.
  if (!isMac && key === "c")
    return () => navigator.clipboard.writeText(term.getSelection()).catch(alertish);
  if (!isMac && key === "v")
    return () =>
      navigator.clipboard
        .readText()
        .then((t) => term.paste(t))
        .catch(alertish);
  return null;
}

// One step of text size for every terminal. Saved once the keys stop: two saves in
// flight and the second is refused as a file that moved under it.
let fontSave;
function zoomTerminals(step) {
  const { font_size, ...rest } = prefs;
  prefs = step ? { ...rest, font_size: Math.min(32, Math.max(8, termFont() + step)) } : rest;
  restyleTerminals();
  clearTimeout(fontSave);
  fontSave = setTimeout(() => invoke("save_settings", { next: prefs }).catch(alertish), 500);
}

// Select All up to the last line with something on it, as a Mac terminal does. xterm's
// own paints every empty row under the prompt and copies them as blank lines.
function selectWritten(term) {
  const buf = term.buffer.active;
  let last = buf.length - 1;
  while (last > 0 && !buf.getLine(last)?.translateToString(true)) last--;
  term.selectLines(0, last);
}

function makeTerm(host) {
  const term = new Terminal({
    fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--mono").trim(),
    fontSize: termFont(),
    // Exactly 1: any leading turns box-drawing rules in a TUI into dashed lines.
    lineHeight: 1,
    cursorBlink: true,
    cursorInactiveStyle: "outline",
    allowTransparency: true,
    scrollback: 5000,
    // The search addon's highlights are decorations, still a proposed API: without this
    // every search throws before it selects anything.
    allowProposedApi: true,
    theme: theme(),
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  const search = new SearchAddon.SearchAddon();
  term.loadAddon(search);
  // A link in output was written by the remote host: it goes to the browser through
  // `open_link` (http(s) only), never to a webview of ours.
  term.loadAddon(
    new WebLinksAddon.WebLinksAddon((_, uri) => {
      invoke("open_link", { url: uri }).catch(alertish);
    }),
  );
  term.attachCustomKeyEventHandler((e) => {
    if (e.type !== "keydown") return true;
    const act = termKey(e, term);
    if (!act) return true;
    // Stopped here, or the window's handler reads ⌘+ on a German layout as ⌘].
    e.preventDefault();
    e.stopPropagation();
    act();
    return false;
  });
  term.open(host);
  // ⌘A never reaches the handler above: the page is a child webview (`unstable`), and
  // wry hands a child's ⌘-keys to the menu first. Its Select All lands on xterm's hidden
  // textarea, and WebKit asks that with a selectstart before selecting nothing.
  term.textarea.addEventListener("selectstart", (e) => {
    e.preventDefault();
    selectWritten(term);
  });
  // No webgl renderer: it leaves the previous frame behind on the transparent
  // background macOS vibrancy needs.
  return { term, fit, search };
}

// Focus the live tab for `key` if there is one. `key` is what the tab is of: a task, a
// shell or a web url, so a device whose url changed is a different page.
function showOpen(key) {
  const open = [...sessions.values()].find((s) => s.key === key && !s.dead);
  if (!open) return false;
  activeId = open.id;
  showTab();
  renderTabs();
  renderTree();
  return true;
}

// A broadcast group: sessions sharing one tab and, while `on`, their keystrokes. Each
// session points at the object, so a closing pane removes itself from `live`.
let nextGid = 1;
function makeBroadcast(names) {
  return { gid: nextGid++, on: true, names, live: new Set() };
}
const inActive = (s) => {
  if (activeId === null) return false;
  const a = sessions.get(activeId);
  return s === a || (a?.bcast && s.bcast === a.bcast);
};

// Every marked ssh device as a pane in one broadcast tab. Non-ssh marks are named in a
// pill and skipped: the grid holds terminals only.
async function openBroadcast(marks) {
  const ssh = marks.filter((m) => m.ssh);
  const skipped = marks.filter((m) => !m.ssh);
  if (ssh.length < 2) return alertish("A broadcast needs two or more ssh devices.");
  if (skipped.length) {
    flash(`Broadcasting to ${ssh.length} · left out: ${skipped.map((m) => m.name).join(", ")}`);
  }
  const b = makeBroadcast(ssh.map((m) => m.name));
  // In parallel, or a twelve-device broadcast pops in one pane per frame.
  await Promise.all(ssh.map((j) => openSession(j.name, null, b)));
  const first = [...sessions.values()].find((s) => s.bcast === b);
  if (first) {
    activeId = first.id;
    showTab();
    renderTabs();
  }
}

// "ping-on" is ping until ^C: offered once a ping has finished, never as a first click.
const taskLabel = (task) => (task === "ping-on" ? "ping ∞" : task);

// A terminal tab. `task` ("ping", "ping-on" or "trace") runs a check instead of a shell,
// on the jump host when there is one. `bcast` joins the session to a broadcast group.
async function openSession(name, task = null, bcast = null) {
  // A broadcast pane never counts as "already open": two grids are two grids.
  const key = bcast ? `bcast:${bcast.gid}:${name}` : `term:${task ?? ""}:${name}`;
  if (!bcast && showOpen(key)) return;
  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost" + (bcast ? " bcasthost" : "");
  if (bcast) {
    host.dataset.name = name;
    host.addEventListener(
      "mousedown",
      () => {
        activeId = id;
        showTab();
        renderTabs();
      },
      true,
    );
  }
  termsEl.append(host);

  const { term, fit, search } = makeTerm(host);

  const s = {
    id,
    name,
    task,
    kind: "term",
    key,
    term,
    fit,
    search,
    host,
    dead: false,
    unlisten: [],
    bcast,
  };
  sessions.set(id, s);
  if (bcast) bcast.live.add(id);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();
  // xterm needs a laid-out pane before fit(); sizing twice sends the shell a SIGWINCH
  // mid-login and leaves a TUI painting on a reflowed grid.
  await new Promise((r) => requestAnimationFrame(r));
  fit.fit();

  term.onData((d) => {
    // A dead tab keeps the keyboard: Enter dials the same device again.
    if (s.dead) {
      const again = d === "\r" ? task : d === "c" && task === "ping" ? "ping-on" : undefined;
      if (again !== undefined) {
        dropTab(id);
        openSession(name, again, s.bcast);
      }
      return;
    }
    // Broadcast on: every live sibling gets the keystroke. Off: only the focused pane.
    if (s.bcast?.on) {
      for (const other of s.bcast.live) {
        invoke("write_session", { id: other, data: d }).catch(() => {});
      }
      return;
    }
    invoke("write_session", { id, data: d }).catch(() => {});
  });
  term.onResize(({ cols, rows }) => invoke("resize_session", { id, cols, rows }).catch(() => {}));

  // Our banner is cleared by the first byte from the far end, so a login that draws
  // with cursor moves gets a clean screen. An error we wrote on the way in stays.
  let ours = true;
  term.write(`\x1b[2m── ${task ? `${taskLabel(task)} ` : ""}${name}… ──\x1b[0m\r\n`);

  try {
    s.unlisten.push(
      await listen(`pty:${id}`, (e) => {
        if (ours) {
          ours = false;
          term.write("\x1b[2J\x1b[3J\x1b[H");
        }
        term.write(e.payload);
      }),
    );
    s.unlisten.push(
      await listen(`pty-exit:${id}`, (e) => {
        s.dead = true;
        s.bcast?.live.delete(id);
        term.write(
          `\r\n\x1b[2m── ${task ? taskLabel(task) : "ssh"} exited (${e.payload}) · ⏎ to ${task ? "run it again" : "reconnect"}${task === "ping" ? " · c to ping until ^C" : ""} · ${chord("w")} to close ──\x1b[0m\r\n`,
        );
        renderTabs();
        renderTree();
      }),
    );
  } catch (err) {
    // `listen` needs a capability in src-tauri/capabilities; without it the session
    // opens and sits there mute.
    s.dead = true;
    term.write(`\x1b[31mcould not subscribe to the session: ${String(err)}\x1b[0m\r\n`);
    renderTabs();
    return;
  }

  try {
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

// A live tab: a connected shell or desktop. Closing one is not asked about - it is
// one deliberate click on one session - but closing several at once is.
const isLive = (s) => !s.dead && ((s.kind === "term" && !s.task) || s.kind === "rdp");
const liveSessions = () => [...sessions.values()].filter(isLive);

function closeSession(id) {
  const s = sessions.get(id);
  if (!s) return;
  const closer = { rdp: "close_rdp_session", web: "close_web_view" }[s.kind] ?? "close_session";
  invoke(closer, { id }).catch(() => {});
  dropTab(id);
}

// Remove the tab without telling the far end: it has already gone, or never started.
function dropTab(id) {
  const s = sessions.get(id);
  if (!s) return;
  if (s.master != null) invoke("close_session", { id: s.master }).catch(() => {});
  clearInterval(s.wait);
  clearTimeout(s.noteTimer);
  clearTimeout(s.checkTimer);
  s.unlisten.forEach((f) => f());
  s.term?.dispose();
  s.host.remove();
  sessions.delete(id);
  s.bcast?.live.delete(id);
  if (activeId === id) {
    // Stay inside the same broadcast group when one of its panes closes.
    const sibling = s.bcast && [...sessions.values()].find((x) => x.bcast === s.bcast);
    activeId = sibling ? sibling.id : ([...sessions.keys()].pop() ?? null);
  }
  showTab();
  renderTabs();
  renderTree();
}

// activeId === null is the "All jacks" tab; anything else is a session.
function showTab() {
  // The find bar searches one session's scrollback, so it does not follow you.
  if (!findEl.hidden) closeFind();
  browseEl.hidden = activeId !== null;
  termsEl.hidden = activeId === null;
  const grid = sessions.get(activeId)?.bcast ?? null;
  termsEl.classList.toggle("grid", grid !== null);
  termsEl.classList.toggle("bcast-on", !!grid?.on);
  // Nearest-square grid: 4 becomes 2×2, 6 becomes 3×2.
  if (grid) {
    const n = [...sessions.values()].filter((s) => s.bcast === grid).length;
    termsEl.style.setProperty("--cols", Math.max(1, Math.ceil(Math.sqrt(n))));
  } else {
    termsEl.style.removeProperty("--cols");
  }
  for (const s of sessions.values()) {
    s.host.hidden = !inActive(s);
    s.host.classList.toggle("focused", grid !== null && s.id === activeId);
  }
  if (activeId !== null) {
    // Every visible pane refits after the grid template lands.
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

// Where the visible web tab is, or null. Tooltips ask before drawing under it.
function webViewRect() {
  const s = sessions.get(activeId);
  if (!s || s.kind !== "web" || s.failed || modalOpen()) return null;
  const r = s.host.getBoundingClientRect();
  return r.width > 0 ? r : null;
}

// A child webview is an OS view above the page and ignores CSS, so it is either sized
// exactly over its host div or sized to nothing.
function placeWebViews() {
  for (const s of sessions.values()) {
    if (s.kind !== "web") continue;
    const r = s.host.getBoundingClientRect();
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

// One observer on every OVERLAYS() element, so a new overlay cannot forget to move
// the webview out of the way.
let overlayWatch = null;
function watchOverlays() {
  if (overlayWatch) return;
  overlayWatch = new MutationObserver(placeWebViews);
  for (const el of OVERLAYS()) {
    overlayWatch.observe(el, { attributes: true, attributeFilter: ["hidden"] });
  }
}

// How long a page gets to paint before we go looking for a reason. Only a tab that
// showed nothing is ever explained.
const WEB_PATIENCE = 5000;

// Our own check is a *prediction*, and it is made with rustls, which cannot see the
// per-host waiver `security add-trusted-cert -e hostnameMismatch` writes. So a
// certificate you trusted still reads as untrusted here while the webview loads the
// page perfectly well. A page that rendered outranks any prediction: arm the check,
// and let `web-load` cancel it.
function armWebCheck(s, url) {
  clearTimeout(s.checkTimer);
  s.painted = false;
  // The page this tab is waiting on. A check for the one before it is still in flight
  // on a redirect, and its answer would otherwise take this one's timer and then
  // explain the wrong url - which is the url the panel's Trust button fetches.
  s.checkUrl = url;
  s.checkTimer = setTimeout(() => checkWeb(s, url), WEB_PATIENCE);
}

function sameOrigin(a, b) {
  try {
    return new URL(a).origin === new URL(b).origin;
  } catch {
    return false;
  }
}

// The page painted, so whatever we were about to explain isn't true. Also clears a
// panel already up: a slow device that beat the timer must not keep the apology.
function webLoaded(s) {
  clearTimeout(s.checkTimer);
  // A check still in flight resolves after this and would arm the silence timer over a
  // page that is up; it reads this rather than a timer that has already fired.
  s.painted = true;
  if (!s.failed) return;
  s.failed = s.failedUrl = null;
  s.dead = false;
  s.host.innerHTML = "";
  renderTabs();
  placeWebViews();
}

// How much longer a page gets when the check found nothing to explain. Its own
// certificate was waived here once, so `web_check` only asks whether the device answers
// and says fine about a page the webview is refusing - which is what a device updated
// since looks like, because the certificate trusted then is not the one it serves now.
const WEB_SILENCE = 10000;

function checkWeb(s, url) {
  invoke("web_check", { url }).then(
    () => {
      if (s.checkUrl !== url || s.painted) return;
      s.checkTimer = setTimeout(
        () =>
          webFailed(
            s,
            url,
            `"${url}" opened nothing and gave no reason - if the device has been ` +
              `updated or rebuilt since you trusted it, it is serving a new certificate ` +
              `this machine doesn't trust yet`,
          ),
        WEB_SILENCE,
      );
    },
    (e) => webFailed(s, url, String(e)),
  );
}

function webFailed(s, url, why) {
  if (!sessions.has(s.id) || s.failed || s.painted || s.checkUrl !== url) return;
  s.dead = true;
  s.failed = why;
  s.failedUrl = url;
  renderTabs();
  showWebFailure(s);
}

// Our own panel takes the pane; the webview is shrunk away by placeWebViews().
function showWebFailure(s) {
  const cert = s.failed.includes("certificate");
  s.host.innerHTML = `<div class="panefail">
    <p class="why">${esc(s.failed)}</p>
    ${
      cert
        ? `<p class="fix">A device reached by its address has a certificate naming
      something else, and that never matches. <b>Trust it</b> hands the certificate to
      macOS the way the browser's "Always trust" does - macOS asks for your password,
      and the page opens here from then on.</p>`
        : ""
    }
    <div class="btns">
      ${
        cert
          ? `<button type="button" class="primary" data-web-cert="${esc(s.failedUrl)}">
        ${icon("check")}Trust it</button>`
          : ""
      }
      <button type="button" class="ghost" data-web-browser="${esc(s.name)}">
        ${icon("external-link")}Open in browser</button>
      ${/* Skips only our check; the webview still judges the certificate itself. */ ""}
      <button type="button" class="ghost" data-web-trust="${esc(s.failedUrl)}">
        ${icon("globe")}Skip the check</button>
    </div>
  </div>`;
  placeWebViews();
}

termsEl.addEventListener("click", async (e) => {
  const open = e.target.closest("[data-web-browser]");
  if (open) return invoke("open_url", { name: open.dataset.webBrowser }).catch(alertish);

  const cert = e.target.closest("[data-web-cert]");
  const trust = e.target.closest("[data-web-trust]");
  const btn = cert ?? trust;
  if (!btn) return;
  const s = [...sessions.values()].find((x) => x.host.contains(btn));
  if (!s) return;

  try {
    if (cert) {
      // Show the certificate before asking: it was fetched over an unverified
      // connection, so this is the moment a man in the middle would be trusted.
      const c = await invoke("web_cert", { url: cert.dataset.webCert });
      // What is trusted is the device's own signer when it sent one, so that a
      // certificate it renews on an update is still trusted. Say so: it is a wider
      // answer than the one certificate the page is serving today.
      const scope = c.ca
        ? `This is the device's own signing certificate, not the one it is serving ` +
          `today - trusting it covers the certificates it issues for this device.\n\n`
        : "";
      const ok = await ask(
        `Trust this certificate?\n\n${scope}${c.subject}\nissued by ${c.issuer}\n` +
          `expires ${c.expires}\nSHA-256 ${c.fingerprint}`,
        null,
        "Trust it",
      );
      if (!ok) return;
      await invoke("web_trust_cert", { url: cert.dataset.webCert });
    } else {
      await invoke("web_trust", { url: trust.dataset.webTrust });
    }
  } catch (err) {
    return alertish(err);
  }

  // Reload: the webview judged the certificate before the trust store changed.
  const name = s.name;
  closeSession(s.id);
  openWebSession(name);
});

// ── files ──────────────────────────────────────────────────────────────────
// sftp in a tab. Ordinary HTML in our own webview, unlike the web tab.
async function openFilesSession(name) {
  used(name);
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
    // The server's own answer to where that took us, so the bar shows a real path.
    const at = await invoke("sftp_ls", { name: s.name, path: s.cwd });
    s.cwd = at.path;
    s.entries = at.entries;
    s.dead = false;
  } catch (e) {
    s.dead = true;
    s.entries = null;
    s.error = String(e);
    // Denied means it needs a tty to answer a password in, so the tab opens its own.
    if (s.error.includes("Permission denied") && !s.master) return signIn(s);
  }
  renderTabs();
  renderFiles(s);
}

const fileSize = (n) => {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024;
    i++;
  }
  return `${i === 0 ? n : n.toFixed(1)} ${u[i]}`;
};

// macOS TCC answers Desktop, Documents and Downloads with an empty directory rather
// than an error, so an empty listing there says so.
function emptyNote(s) {
  const tcc = /^\/Users\/[^/]+\/(Desktop|Documents|Downloads)\/?$/.test(s.cwd);
  if (!tcc) return `<p class="fempty">Nothing here.</p>`;
  // The setting lives on the machine running sftp-server, so the button only appears
  // when that machine is this one.
  const host = all.find((j) => j.name === s.name)?.host ?? "";
  const here = /^(localhost|127\.0\.0\.1|::1)$/i.test(host);
  return `<p class="fempty">Nothing here. macOS keeps Desktop, Documents and Downloads
    out of an ssh session's reach until <b>sftp-server</b> has Full Disk Access${
      here ? "" : ` on ${esc(s.name)}`
    }.</p>
    ${
      here
        ? `<div class="btns"><button type="button" class="ghost" data-files="fda">
      ${icon("settings")}Open Full Disk Access</button></div>`
        : ""
    }`;
}

function renderFiles(s) {
  if (!s.entries) {
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
  // Folders first, then names.
  const rows = [...s.entries].sort((a, b) =>
    a.dir === b.dir ? a.name.localeCompare(b.name) : a.dir ? -1 : 1,
  );

  s.host.innerHTML = `<div class="files">
    <div class="fpath">
      <button type="button" class="flat" data-up="1" data-tip="Up a folder">${icon("chevron-right")}</button>
      <span class="mono">${esc(s.cwd)}</span>
      <span class="fhint">${icon("download")}drop files here to upload</span>
      <button type="button" class="flat" data-files="mkdir" data-tip="New folder"
        data-tip-at="right">${icon("folder-plus")}</button>
    </div>
    <div class="flist">${rows.length ? "" : emptyNote(s)}${rows
      .map(
        (e, i) => `
      <div class="frow" data-fi="${i}" data-dir="${e.dir}">
        ${icon(e.dir ? "folder" : "file-pen-line")}
        <span class="fname">${esc(e.name)}</span>
        <span class="fsize">${e.dir ? "" : esc(fileSize(e.size))}</span>
        <span class="fwhen">${esc(e.modified)}</span>
      </div>`,
      )
      .join("")}</div>
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

// `ssh -N` on a pty in the pane, so a password or host-key question is answered where
// it was asked. The connection it leaves behind is what every sftp call here rides.
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
    s.unlisten.push(
      await listen(`pty-exit:${id}`, async () => {
        if (s.master !== id || s.entries) return;
        // ControlPersist backgrounds the client once it has authenticated, so the
        // socket, not the exit, says whether the sign-in worked.
        await new Promise((r) => setTimeout(r, 400));
        if (await invoke("sftp_ready", { name: s.name }).catch(() => false)) return connected(s);
        failedSignIn(s, "that sign-in didn't finish");
      }),
    );
    term.write(`\x1b[2m── signing in to ${s.name} for files ──\x1b[0m\r\n`);
    await invoke("open_master", { id, name: s.name, cols: term.cols, rows: term.rows });
  } catch (e) {
    return failedSignIn(s, String(e));
  }
  watchConnection(s);
}

// Poll for the control socket for two minutes: it appears the moment ssh authenticates.
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

// Back to the panel with the buttons; a shell opened by hand can still connect this tab.
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

// A note in the pane's path bar; alertish() is for errors, and most of these are not.
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
  } catch (e) {
    fileNote(s, String(e), true);
  }
}

// mkdir, rename and remove, each followed by a re-list: the far end decides whether
// it worked.
async function fileEdit(s, op, path, to = "") {
  try {
    await invoke("sftp_edit", { name: s.name, op, path, to });
  } catch (e) {
    return fileNote(s, String(e), true);
  }
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
  // sftp only removes an empty folder, so the question names which kind it is.
  const what = entry.dir ? `the folder "${entry.name}"` : `"${entry.name}"`;
  if (!(await ask(`Delete ${what} on ${s.name}?`, null, "Delete"))) return;
  fileEdit(s, entry.dir ? "rmdir" : "rm", `${s.cwd}/${entry.name}`);
}

// "Edit here": download, open with the desktop's default app, and upload on each save.
// The watch is a thread in Rust, because an editor outlives the tab.
async function editFile(s, name) {
  fileNote(s, `Opening ${name}…`);
  try {
    await invoke("sftp_open", { name: s.name, remote: `${s.cwd}/${name}` });
    fileNote(s, `${name} is open - saving it puts it back`);
  } catch (e) {
    fileNote(s, String(e), true);
  }
}

// Save-back notices arrive from a thread with no tab of its own, so the note finds one,
// or goes through alertish() when the tab is gone and something went wrong.
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
  } catch {
    /* no capability means no notice, not a broken tab */
  }
}

// Real paths from the webview's own drop event; a file picker would need a plugin.
async function watchDrops() {
  if (watchDrops.on) return;
  watchDrops.on = true;
  try {
    await listen("tauri://drag-drop", async (e) => {
      const s = sessions.get(activeId);
      const paths = e.payload?.paths ?? [];
      if (!paths.length) return;
      // A remote desktop sees one folder of ours as a drive; a drop lands there.
      if (s?.kind === "rdp")
        return invoke("rdp_drop", { paths }).then((said) => flash(said), alertish);
      if (s?.kind !== "sftp") return;
      fileNote(s, `Uploading ${paths.length} file${paths.length === 1 ? "" : "s"}…`);
      let failed = null;
      for (const local of paths) {
        try {
          await invoke("sftp_put", { name: s.name, local, remoteDir: s.cwd });
        } catch (err) {
          failed = String(err);
          break;
        }
      }
      await listFiles(s);
      if (failed) fileNote(s, failed, true);
    });
  } catch {
    /* no capability means no drag-drop, not a broken tab */
  }
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
      id,
      name,
      x: r.left,
      y: r.top,
      width: r.width,
      height: r.height,
    });
  } catch (e) {
    // Nothing was shown, so there is no tab worth keeping.
    dropTab(id);
    return alertish(e);
  }

  // Follow redirects: a Synology's http port redirects to https in JavaScript, so the
  // certificate that blanks the tab belongs to a url no http client would see.
  try {
    s.unlisten.push(
      await listen(`web-nav:${id}`, (e) => {
        // Each navigation is a fresh page to be patient with; the one that paints
        // cancels the check for all of them. Except one on the origin that just
        // painted: wry reports every frame's navigation but only the main frame's
        // `web-load`, so a Proxmox console or a DSM widget iframe (or a `#v1:...`
        // fragment) re-armed into Trust it again with nothing to cancel it. A
        // certificate belongs to the origin, and that one was just accepted.
        if (s.painted && sameOrigin(e.payload, s.checkUrl)) return;
        armWebCheck(s, e.payload);
      }),
    );
    s.unlisten.push(await listen(`web-load:${id}`, () => webLoaded(s)));
  } catch {
    /* no capability means no redirect notice, not a broken tab */
  }

  armWebCheck(s, url);
}

function renderTabs() {
  // The browse tab is the crumb: the selected folder and its count.
  const label = group === null ? "All jacks" : groupLabel().split("/").join(" / ");
  const browse = `<div class="tab" data-id="" aria-selected="${activeId === null}">
      ${icon("layers")}<span class="lbl">${esc(label)}</span><span class="n">${shown.length}</span></div>`;
  // A broadcast group is one chip, selected whenever any of its panes has focus.
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
    // A shell tab wears its device's mark in readable()'s colour, never a raw brand hex.
    const os = s.kind === "term" && !s.task ? all.find((j) => j.name === s.name)?.os : undefined;
    const tint = os && osColor(os);
    chips.push(`<div class="tab ${s.dead ? "dead" : ""}" data-id="${s.id}" aria-selected="${s.id === activeId}">
      <span class="dot ${s.dead ? "down" : "up"}"></span>
      ${os ? `<span class="tabkind"${tint ? ` style="color:${esc(tint)}"` : ""}>${osIcon(os)}</span>` : ""}
      ${s.kind === "rdp" ? `<span class="tabkind">${icon("monitor")}</span>` : ""}
      ${s.kind === "web" ? `<span class="tabkind">${icon("globe")}</span>` : ""}
      ${s.kind === "sftp" ? `<span class="tabkind">${icon("folder")}</span>` : ""}
      ${s.task ? `<span class="tabkind">${icon(s.task === "trace" ? "waypoints" : "plug")}</span>` : ""}
      <span class="lbl">${esc(s.task ? `${taskLabel(s.task)} ${labelOf(s.name)}` : labelOf(s.name))}</span>
      ${
        s.kind === "web" && webextRunning && !s.dead
          ? `<span class="x" data-bw="${s.id}" data-tip="Fill · right-click for Bitwarden">${icon("key-round")}</span>`
          : ""
      }
      <span class="x" data-close="${s.id}" data-tip="Close  ${chord("w")}">${icon("x")}</span>
    </div>`);
  }
  tabsEl.innerHTML = browse + chips.join("");
  // Here rather than in render(): the extension starts after the first paint.
  const vault = $("vault");
  vault.hidden = !webextRunning;
  vault.innerHTML = `${icon("key-round")}Vault`;
  vault.dataset.tip = "Open Bitwarden";
  // Scroll the active tab into view by hand: scrollIntoView walks up to any scrollable
  // ancestor and took the detail pane off the side of the window with it.
  const active = tabsEl.querySelector('[aria-selected="true"]');
  if (active) {
    const right = active.offsetLeft + active.offsetWidth;
    if (active.offsetLeft < tabsEl.scrollLeft) tabsEl.scrollLeft = active.offsetLeft;
    else if (right > tabsEl.scrollLeft + tabsEl.clientWidth) {
      tabsEl.scrollLeft = right - tabsEl.clientWidth;
    }
  }
  markTabOverflow();
  rememberTabs();
}

// Shells, pages and file listings come back on the next launch. Stored in localStorage
// like `recent`: a habit, not part of the list.
function rememberTabs() {
  const open = [...sessions.values()]
    .filter((s) => !s.dead && !s.bcast && !s.task && s.kind !== "rdp")
    .map((s) => ({ kind: s.kind, name: s.name }));
  remember("tabs", JSON.stringify(open));
}

// The strip hides its scrollbar, so fade whichever edge still has tabs beyond it.
function markTabOverflow() {
  const room = tabsEl.scrollWidth - tabsEl.clientWidth;
  // A sub-pixel layout leaves a fraction of scrollable width on a strip that fits.
  tabsEl.classList.toggle("more-l", room > 1 && tabsEl.scrollLeft > 1);
  tabsEl.classList.toggle("more-r", room > 1 && tabsEl.scrollLeft < room - 1);
}
tabsEl.addEventListener("scroll", markTabOverflow, { passive: true });

// The key on a web tab. A click fills, the way ⌘⇧L does in a browser, so a login in
// steps is one click per step; the popup closed itself after every fill. Right-click
// opens Bitwarden itself, for signing in and picking a login by hand. Whatever it
// shows hangs from the key.
// The toolbar's key carries no tab: Bitwarden opens on whatever page is in front, or
// on nothing, which is the vault as a vault rather than a form to fill.
function bitwardenKey(el, popup) {
  const r = el.getBoundingClientRect();
  const at = { x: r.left, y: r.top, width: r.width, height: r.height };
  const id = el.dataset.bw ? +el.dataset.bw : null;
  invoke("webext_key", { id, ...at, popup }).catch(alertish);
}
tabsEl.addEventListener("contextmenu", (e) => {
  const bw = e.target.closest("[data-bw]");
  if (!bw) return;
  e.preventDefault();
  // Before the document's own handler, which would draw the app's menu over it.
  e.stopPropagation();
  bitwardenKey(bw, true);
});

tabsEl.addEventListener("click", (e) => {
  const bw = e.target.closest("[data-bw]");
  if (bw) return bitwardenKey(bw, false);
  const close = e.target.closest("[data-close]")?.dataset.close;
  if (close) return closeSession(+close);
  const bclose = e.target.closest("[data-bclose]")?.dataset.bclose;
  if (bclose) return closeBroadcast(+bclose);
  const bmute = e.target.closest("[data-bmute]")?.dataset.bmute;
  if (bmute) return toggleBroadcast(+bmute);
  const bcast = e.target.closest("[data-gid]");
  if (bcast) {
    const gid = +bcast.dataset.gid;
    // The first live pane, or the first pane if every one has died.
    const first =
      [...sessions.values()].find((s) => s.bcast?.gid === gid && !s.dead) ??
      [...sessions.values()].find((s) => s.bcast?.gid === gid);
    if (first) {
      activeId = first.id;
      showTab();
      renderTabs();
    }
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
  showTab();
  renderTabs();
}

async function closeBroadcast(gid) {
  const panes = [...sessions.values()].filter((s) => s.bcast?.gid === gid);
  const live = panes.filter(isLive).length;
  if (live > 1 && !(await ask(`Close ${live} live sessions?`, null, "Close"))) return;
  for (const s of panes) closeSession(s.id);
}

addEventListener("resize", () => {
  for (const s of sessions.values()) if (inActive(s)) s.fit?.fit();
  placeWebViews();
  markTabOverflow();
});

function cycleSession(d) {
  // One step per chip: a broadcast group is one tab in the strip.
  const stops = [null];
  const seen = new Set();
  for (const s of sessions.values()) {
    if (!s.bcast) {
      stops.push(s.id);
      continue;
    }
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
// Deliberately not in OVERLAYS(): modalOpen() would hand the window's chords to the
// box and shrink a web tab away. It lives inside #terms and takes Escape on its own input.
const findEl = $("find"),
  findQ = $("find-q"),
  findN = $("find-n");
// Which session the bar is searching. Not `activeId`: closing the bar has to clear the
// highlights off the terminal it was searching, and switching tabs is when it closes.
let findFor = null;

// Match colours read off the page, so they come from `:root` like every other colour.
const findColors = () => {
  const css = getComputedStyle(document.body);
  const v = (n) => css.getPropertyValue(n).trim();
  return {
    decorations: {
      matchBackground: v("--term-match"),
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
  if (!s?.search || !q) {
    findN.textContent = "";
    return;
  }
  const opts = { ...findColors(), incremental: !back };
  const found = back ? s.search.findPrevious(q, opts) : s.search.findNext(q, opts);
  findN.textContent = found ? "" : "no match";
}

// Only where there is scrollback to search: a desktop, a page or a file list has none.
function toggleFind() {
  const s = sessions.get(activeId);
  if (!s?.search) return;
  if (!findEl.hidden) return closeFind();
  findFor = activeId;
  // Icons are painted here because render() never touches the session pane.
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
  const s = sessions.get(findFor);
  findFor = null;
  s?.search?.clearDecorations();
  if (s && s.id === activeId) s.term?.focus();
}

findQ.addEventListener("input", () => findRun());
findQ.addEventListener("keydown", (e) => {
  // The input has the keyboard here, so these never reach the window's own handler.
  if (e.key === "Escape") {
    e.preventDefault();
    closeFind();
  } else if (e.key === "Enter") {
    e.preventDefault();
    findRun(e.shiftKey);
  }
});
$("find-prev").addEventListener("click", () => findRun(true));
$("find-next").addEventListener("click", () => findRun());
$("find-x").addEventListener("click", closeFind);

// ── remote desktop tabs ────────────────────────────────────────────────────
// The pane is a <canvas> that Rust paints dirty rectangles onto; the other half is
// src-tauri/src/rdp_session.rs.

// The browser names keys; RDP wants PC/AT set 1 scancodes. 0xE0__ marks the extended
// ones, which is what Scancode::from_u16 looks for.
const SCANCODES = {
  Escape: 0x01,
  Digit1: 0x02,
  Digit2: 0x03,
  Digit3: 0x04,
  Digit4: 0x05,
  Digit5: 0x06,
  Digit6: 0x07,
  Digit7: 0x08,
  Digit8: 0x09,
  Digit9: 0x0a,
  Digit0: 0x0b,
  Minus: 0x0c,
  Equal: 0x0d,
  Backspace: 0x0e,
  Tab: 0x0f,
  KeyQ: 0x10,
  KeyW: 0x11,
  KeyE: 0x12,
  KeyR: 0x13,
  KeyT: 0x14,
  KeyY: 0x15,
  KeyU: 0x16,
  KeyI: 0x17,
  KeyO: 0x18,
  KeyP: 0x19,
  BracketLeft: 0x1a,
  BracketRight: 0x1b,
  Enter: 0x1c,
  ControlLeft: 0x1d,
  KeyA: 0x1e,
  KeyS: 0x1f,
  KeyD: 0x20,
  KeyF: 0x21,
  KeyG: 0x22,
  KeyH: 0x23,
  KeyJ: 0x24,
  KeyK: 0x25,
  KeyL: 0x26,
  Semicolon: 0x27,
  Quote: 0x28,
  Backquote: 0x29,
  ShiftLeft: 0x2a,
  Backslash: 0x2b,
  KeyZ: 0x2c,
  KeyX: 0x2d,
  KeyC: 0x2e,
  KeyV: 0x2f,
  KeyB: 0x30,
  KeyN: 0x31,
  KeyM: 0x32,
  Comma: 0x33,
  Period: 0x34,
  Slash: 0x35,
  ShiftRight: 0x36,
  NumpadMultiply: 0x37,
  AltLeft: 0x38,
  Space: 0x39,
  CapsLock: 0x3a,
  F1: 0x3b,
  F2: 0x3c,
  F3: 0x3d,
  F4: 0x3e,
  F5: 0x3f,
  F6: 0x40,
  F7: 0x41,
  F8: 0x42,
  F9: 0x43,
  F10: 0x44,
  NumLock: 0x45,
  ScrollLock: 0x46,
  Numpad7: 0x47,
  Numpad8: 0x48,
  Numpad9: 0x49,
  NumpadSubtract: 0x4a,
  Numpad4: 0x4b,
  Numpad5: 0x4c,
  Numpad6: 0x4d,
  NumpadAdd: 0x4e,
  Numpad1: 0x4f,
  Numpad2: 0x50,
  Numpad3: 0x51,
  Numpad0: 0x52,
  NumpadDecimal: 0x53,
  F11: 0x57,
  F12: 0x58,
  ControlRight: 0xe01d,
  AltRight: 0xe038,
  NumpadDivide: 0xe035,
  NumpadEnter: 0xe01c,
  Home: 0xe047,
  ArrowUp: 0xe048,
  PageUp: 0xe049,
  ArrowLeft: 0xe04b,
  ArrowRight: 0xe04d,
  End: 0xe04f,
  ArrowDown: 0xe050,
  PageDown: 0xe051,
  Insert: 0xe052,
  Delete: 0xe053,
  MetaLeft: 0xe05b,
  MetaRight: 0xe05c,
};

// Credentials for the window's lifetime only, never written anywhere.
const rdpCreds = new Map();

async function openRdpSession(name) {
  const j = all.find((x) => x.name === name);
  if (!j) return;
  // Before the credentials are asked for: a second desktop would be a second login.
  const key = `rdp:${name}`;
  if (showOpen(key)) return;

  let creds = rdpCreds.get(name);
  if (!creds) {
    // The username is asked for even with one in the config: a Windows box is usually
    // reached as a different account than ssh uses.
    creds = await ask(`Sign in to ${j.host}`, "", "Connect", "password", j.user ?? "");
    if (!creds) return;
  }

  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost rdphost";
  const canvas = document.createElement("canvas");
  canvas.tabIndex = 0; // so it can take the keyboard at all
  host.append(canvas);
  termsEl.append(host);
  const ctx = canvas.getContext("2d");
  watchDrops();

  const s = { id, name, kind: "rdp", key, canvas, host, dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();

  // The desktop is letterboxed into the pane while it is a different size: a resize is
  // a round trip to the server, and the window must not go black in the meantime. The
  // element keeps the picture's aspect so `at()` stays a plain rect ratio.
  const scale = () => {
    // A backgrounded tab has no box, and a canvas sized to nothing would stay that
    // way if the observer didn't fire again on the way back.
    if (!host.clientWidth || !host.clientHeight) return;
    const k = Math.min(host.clientWidth / canvas.width, host.clientHeight / canvas.height);
    canvas.style.width = `${Math.round(canvas.width * k)}px`;
    canvas.style.height = `${Math.round(canvas.height * k)}px`;
  };

  // Tiles arrive before the invoke resolves, and setting canvas.width clears the
  // canvas, so they are held until the server has said how big the desktop is.
  let queued = [];
  const paint = (buf) => {
    const view = new DataView(buf);
    // Records of an 8-byte header (x, y, w, h as little-endian u16), then raw RGBA.
    for (let at = 0; at < buf.byteLength;) {
      const x = view.getUint16(at, true),
        y = view.getUint16(at + 2, true),
        w = view.getUint16(at + 4, true),
        h = view.getUint16(at + 6, true);
      at += 8;
      // x at 0xffff has no pixels behind it: it is the desktop's new size, in line with
      // the tiles so nothing painted before the change is dropped on the floor.
      if (x === 0xffff) {
        canvas.width = w;
        canvas.height = h;
        scale();
        continue;
      }
      ctx.putImageData(new ImageData(new Uint8ClampedArray(buf, at, w * h * 4), w, h), x, y);
      at += w * h * 4;
    }
  };
  const chan = new window.__TAURI__.core.Channel();
  chan.onmessage = (msg) => {
    const buf = msg instanceof ArrayBuffer ? msg : new Uint8Array(msg).buffer;
    queued ? queued.push(buf) : paint(buf);
  };

  // The pane's own size in device pixels, so the desktop fits without being scaled:
  // in CSS pixels a 2x screen draws each one as a 2x2 block and the text goes soft.
  // `scale` is what keeps it from being half the size. Even numbers because the RDP
  // codecs work in 2x2 blocks. Laid out by showTab() above.
  const px = (n) => Math.max(640, Math.min(8192, n * devicePixelRatio)) & ~1;
  const percent = () => Math.round(devicePixelRatio * 100);
  // What the far end was last asked for. The observer below fires once the moment it
  // starts observing, and the pane is still the size the session was opened at.
  let asked = [px(host.clientWidth), px(host.clientHeight), percent()];
  try {
    const screen = await invoke("open_rdp_session", {
      id,
      name,
      user: creds.user,
      password: creds.password,
      width: asked[0],
      height: asked[1],
      scale: asked[2],
      onTile: chan,
    });
    // The server picks the size; asking for one is only a suggestion.
    canvas.width = screen.width;
    canvas.height = screen.height;
    // Every resize tears the session down and rebuilds it, so the pointer is only
    // believed once it has stopped moving. A server without the Display Control
    // channel ignores it and the letterboxing above is all there is.
    // ponytail: a move to a screen with another pixel ratio keeps the old size until
    // the pane is next resized; a matchMedia on the resolution if that shows.
    let settle;
    const ro = new ResizeObserver(() => {
      // A hidden pane measures zero, and asking for that would shrink the desktop to
      // the minimum every time another tab is looked at.
      if (!host.clientWidth || !host.clientHeight) return;
      scale();
      const want = [px(host.clientWidth), px(host.clientHeight), percent()];
      if (want.every((n, i) => n === asked[i])) return;
      asked = want;
      clearTimeout(settle);
      settle = setTimeout(
        () =>
          invoke("rdp_input", {
            id,
            kind: "resize",
            a: want[0],
            b: want[1],
            down: false,
            scale: want[2],
          }).catch(() => {}),
        400,
      );
    });
    ro.observe(host);
    s.unlisten.push(() => {
      ro.disconnect();
      clearTimeout(settle);
    });
    const held = queued;
    queued = null;
    held.forEach(paint);
    rdpCreds.set(name, creds);
  } catch (err) {
    s.dead = true;
    // Rejected credentials must not be remembered, or the next attempt reuses them.
    rdpCreds.delete(name);
    renderTabs();
    // A changed certificate is refused before the password is sent, so the sign-in
    // is kept for the retry. The fingerprint is the one the refusal named: trusting it
    // lets in that certificate and not whatever the host serves by the time we ask.
    const changed = String(err).match(/SHA-256 ([0-9a-f]{64})/);
    if (!changed) return alertish(err);
    const ok = await ask(
      `${j.name} is serving a different certificate than last time.\n\n` +
        `That is expected after Windows renews its own, or the machine was rebuilt - ` +
        `and it is also what someone in between would look like.\n\n` +
        `SHA-256 ${changed[1]}`,
      null,
      "Trust it",
    );
    if (!ok) return;
    try {
      await invoke("rdp_trust", { name, fingerprint: changed[1] });
    } catch (e) {
      return alertish(e);
    }
    dropTab(id);
    rdpCreds.set(name, creds);
    return openRdpSession(name);
  }

  // Without this a hang-up freezes the picture while the tab keeps a live dot.
  try {
    s.unlisten.push(
      await listen(`rdp-exit:${id}`, (e) => {
        s.dead = true;
        renderTabs();
        renderTree();
        if (e.payload) alertish(e.payload);
      }),
    );
  } catch {
    /* no capability means no exit notice, not a broken session */
  }

  const send = (kind, a = 0, b = 0, down = false) =>
    invoke("rdp_input", { id, kind, a, b, down }).catch(() => {});
  // The canvas is letterboxed to fit the pane, so pointer coordinates are scaled back.
  const at = (e) => {
    const r = canvas.getBoundingClientRect();
    return [
      Math.round((e.clientX - r.left) * (canvas.width / r.width)),
      Math.round((e.clientY - r.top) * (canvas.height / r.height)),
    ];
  };
  // One move per frame, not per event: a drag fires a hundred a second, each of them an
  // invoke and a layout read for the rect, and only the last position in a frame is one
  // the far end can act on. A button flushes first, or the click lands where the pointer
  // was a frame ago.
  let pending = null,
    frame = 0;
  const flush = () => {
    cancelAnimationFrame(frame);
    frame = 0;
    if (pending) send("move", ...at(pending));
    pending = null;
  };
  canvas.addEventListener("mousemove", (e) => {
    pending = e;
    frame ||= requestAnimationFrame(flush);
  });
  canvas.addEventListener("mousedown", (e) => {
    canvas.focus();
    flush();
    send("button", e.button, 0, true);
  });
  canvas.addEventListener("mouseup", (e) => {
    flush();
    send("button", e.button, 0, false);
  });
  // Right-click belongs to the far end, but stopping propagation alone skips the
  // document handler's preventDefault and leaves the webview's own Reload menu.
  canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
  });
  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      // A wheel carries no coordinates either: it lands wherever the last move left it.
      flush();
      send("wheel", e.deltaY > 0 ? -120 : 120);
    },
    { passive: false },
  );
  // What the far end believes is held down. macOS delivers no keyup for a key pressed
  // while ⌘ is held, and a chord that moves the focus (⌘K) takes the Meta keyup with
  // it - either way Windows keeps the key down and every letter after it is a
  // shortcut. Releasing on blur and after Meta is what stops that.
  const down = new Set();
  const release = () => {
    for (const code of down) send("key", code, 0, false);
    down.clear();
  };
  for (const [type, pressed] of [
    ["keydown", true],
    ["keyup", false],
  ]) {
    canvas.addEventListener(type, (e) => {
      const code = SCANCODES[e.code];
      if (code === undefined) return;
      // Window chords stay ours; everything else belongs to the remote desktop.
      if (chorded(e) && ["w", "k", "n", "[", "]"].includes(chordKey(e))) return;
      e.preventDefault();
      if (pressed) down.add(code);
      else down.delete(code);
      send("key", code, 0, pressed);
      if (!pressed && (e.code === "MetaLeft" || e.code === "MetaRight")) release();
    });
  }
  canvas.addEventListener("blur", release);

  renderTabs();
  canvas.focus();
}
