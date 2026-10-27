// Session tabs: terminals, file listings, web views and remote desktops, all stacked in
// #terms with only the active one shown. Classic script sharing one global scope with
// the other ui/ files, so anything reaching across files stays inside a function body.
//
// A live session owns the keyboard. The global `keydown` handler returns early when `activeId !==
// null`; every keystroke belongs to ssh. Only window-level chords (⌘K, ⌘N, ⌘D, ⌘W, ⌘[/]) may be
// intercepted, and each one you add is a key someone can no longer send to their remote shell. The
// terminal's own keys are `termKey()` in `sessions.js`: on macOS ⌘ never reached the shell, so ⌘A,
// ⌘+ and ⌘← cost nothing, and ⌥← is *remapped* to readline's `ESC b` rather than taken. On macOS a
// ⌘ key the menu bar has (⌘A, ⌘C, ⌘V, ⌘Z) never reaches a `keydown`: `unstable` makes the page a
// child webview, and wry sends a child's ⌘-keys to the menu first. Catch the menu's effect instead,
// the way ⌘A is a `selectstart` on xterm's textarea. Everything that sends bytes stops at the
// shell's screen: in vim, ⌘←'s `^A` would increment a number.
//
// The find bar is the second thing deliberately *not* in `OVERLAYS()`, for the update pill's
// reason: `modalOpen()` would hand the window's chords to a find box and shrink a web tab away
// every time you looked through a shell. It lives inside `#terms`, so the pane hides it, and it
// takes Escape on its own input. Its chord is ⌘F, and it was the first to be Ctrl+Shift+F elsewhere
// - plain Ctrl+F is readline's forward-char, and a session owns the keyboard. Every chord is that
// now: `chorded(e)` is ⌘ on macOS and Ctrl+Shift off it, and `chordKey(e)` reads punctuation off
// `e.code` because Shift turns `[` into `{`.
//
// A menu only hides a web tab when it reaches over it. `covered()` in `sessions.js` asks each
// overlay for its box, and `showCtx` keeps a menu opened in the sidebar off the page. Sheets and
// the palette cover the window, so they still do.
//
// Xterm needs a laid-out element to size itself, so `fit()` after the pane is visible, not before.
//
// A snippet is typed, never run. `[snippet]` in either vault's file is name = one line of text,
// picked in the palette while a shell is in front and put on the prompt with `term.paste`
// (`typeSnippet` in `sessions.js`), which is the same `onData` a keystroke takes, so a broadcast
// fans it out. A `{?}` or `{?label}` is a blank, asked for through `ask()` before anything is
// typed, once for every pane of a broadcast. It is never followed by Enter, and `snippet_ok()` in
// `patchbay.rs` refuses every control character: that check is the whole reason a colleague's file
// can put text in front of you and cannot run it. It is made on save, on read (a refused one is
// left out and named in a pill, because a shared file never went through a save here) and once more
// in the window. Never add a way for one to press Enter. It stops at the shell's screen like
// everything else that sends bytes, and in a broadcast one pane in an editor stops it for all of
// them.

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
    // The pane behind it is the app's own surface, so the terminal paints no ground. Still
    // --bg at zero alpha, not black: xterm blends the selection over this colour and draws
    // inverse text in it opaque, so black made both dark-on-dark in light mode.
    background: `${v("--bg")}00`,
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
  return s === a || (a?.bcast && s.bcast === a.bcast) || (a?.split && s.split === a.split);
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

// ── split panes ────────────────────────────────────────────────────────────
// Sessions side by side in one tab: any kind, a shell beside a desktop beside a page.
// A split is an object every pane points at, as a broadcast is; one level, panes in a
// row or a column, each with a share of the room that a divider moves.
// ponytail: one level of split; a tree of them if someone wants a 2x2 of mixed panes.
// A split is not remembered across launches: its panes come back as tabs of their own.
// Broadcast panes stay out: their grid is a layout of its own, and their keystrokes go
// to every pane, which a split's must not.
let nextSid = 1;

// Session `id` beside `anchor`, in a split that grows or is made.
function joinSplit(anchor, id, dir = "row") {
  const a = sessions.get(anchor),
    s = sessions.get(id);
  if (!a || !s || a === s || a.bcast || s.bcast) return;
  leaveSplit(s);
  const g = (a.split ??= { sid: nextSid++, dir, ids: [a.id], sizes: [1], last: a.id });
  g.ids.splice(g.ids.indexOf(a.id) + 1, 0, id);
  // Even shares again: a new pane squeezed into a sliver helps nobody.
  g.sizes = g.ids.map(() => 1);
  s.split = g;
  // Its panes together where the split's chip is, so ⌘[ / ⌘] and the strip's order
  // walk through them in the order they sit.
  const order = [...sessions.values()].flatMap((x) =>
    x === a ? g.ids.map((i) => sessions.get(i)) : g.ids.includes(x.id) ? [] : [x],
  );
  sessions.clear();
  for (const x of order) sessions.set(x.id, x);
  activeId = id;
  showTab();
  renderTabs();
}

// Out of its split, which is dissolved when one pane is all that is left.
function leaveSplit(s) {
  const g = s.split;
  if (!g) return;
  const i = g.ids.indexOf(s.id);
  g.ids.splice(i, 1);
  g.sizes.splice(i, 1);
  s.split = null;
  if (g.ids.length === 1) sessions.get(g.ids[0]).split = null;
  if (g.last === s.id) g.last = g.ids[0];
}

// Every pane its own tab again, in the order they sat.
function separateSplit(sid) {
  for (const s of [...sessions.values()].filter((x) => x.split?.sid === sid)) leaveSplit(s);
  showTab();
  renderTabs();
}

async function closeSplit(sid) {
  const panes = [...sessions.values()].filter((s) => s.split?.sid === sid);
  const live = panes.filter(isLive).length;
  if (live > 1 && !(await ask(`Close ${live} live sessions?`, null, "Close"))) return;
  for (const s of panes) closeSession(s.id);
}

// Open a device the way Enter would, and put what opened beside the tab in front. A
// device already open in a tab of its own moves in rather than opening twice.
async function openBeside(name, dir = "row") {
  const anchor = activeId;
  if (anchor === null) return primary(name);
  // A shell is opened fresh, so a second one of the same device can sit beside the first;
  // anything else is one login per device, and an open one moves in.
  const j = all.find((x) => x.name === name);
  const via = ["ssh", "telnet", "serial"].includes(j?.primary) ? j.primary : null;
  if (via) {
    used(name);
    await openSession(name, null, null, false, via, true);
  } else await primary(name);
  if (activeId !== null && activeId !== anchor && sessions.has(anchor)) {
    joinSplit(anchor, activeId, dir);
  }
}

// A divider drag moves the room between the two panes either side of it, never past
// a pane's minimum, and refits them as it goes; a double-click evens the split out.
const PANE_MIN = 140;
termsEl.addEventListener("pointerdown", (e) => {
  const div = e.target.closest(".divider");
  if (!div || e.button !== 0) return;
  const g = sessions.get(activeId)?.split;
  if (!g) return;
  e.preventDefault();
  const i = +div.dataset.at;
  const col = g.dir === "col";
  const [a, b] = [g.ids[i - 1], g.ids[i]].map((id) => sessions.get(id).host);
  const size = (el) => (col ? el.offsetHeight : el.offsetWidth);
  const total = size(a) + size(b),
    share = g.sizes[i - 1] + g.sizes[i];
  const start = col ? e.clientY : e.clientX,
    from = size(a);
  div.setPointerCapture(e.pointerId);
  document.body.classList.add("dragging");
  let frame = 0;
  const onMove = (ev) => {
    const px = Math.min(
      total - PANE_MIN,
      Math.max(PANE_MIN, from + (col ? ev.clientY : ev.clientX) - start),
    );
    g.sizes[i - 1] = (share * px) / total;
    g.sizes[i] = share - g.sizes[i - 1];
    a.style.flexGrow = g.sizes[i - 1];
    b.style.flexGrow = g.sizes[i];
    frame ||= requestAnimationFrame(() => {
      frame = 0;
      for (const id of g.ids) sessions.get(id)?.fit?.fit();
      placeWebViews();
    });
  };
  const end = () => {
    div.removeEventListener("pointermove", onMove);
    div.removeEventListener("lostpointercapture", end);
    document.body.classList.remove("dragging");
  };
  div.addEventListener("pointermove", onMove);
  div.addEventListener("lostpointercapture", end);
});
termsEl.addEventListener("dblclick", (e) => {
  const g = e.target.closest(".divider") && sessions.get(activeId)?.split;
  if (!g) return;
  g.sizes = g.ids.map(() => 1);
  showTab();
});
// A click in a pane gives it the keyboard. Capture, so it lands before the pane's own
// handlers see the click, as a broadcast pane's does.
termsEl.addEventListener(
  "mousedown",
  (e) => {
    const host = e.target.closest(".termhost");
    const s = host && [...sessions.values()].find((x) => x.host === host);
    if (!s?.split || s.id === activeId || !inActive(s)) return;
    activeId = s.id;
    showTab();
    renderTabs();
  },
  true,
);

// "ping-on" is ping until ^C: offered once a ping has finished, never as a first click.
const taskLabel = (task) => (task === "ping-on" ? "ping ∞" : task);

// A terminal tab. `task` ("ping", "ping-on" or "trace") runs a check instead of a shell,
// on the jump host when there is one; "key" installs this machine's key over the chain.
// `bcast` joins the session to a broadcast group.
// `waiting` is a redial from "r": a failed one goes back to waiting rather than stopping.
// `via` is "telnet" or "serial" for a device reached that way instead of ssh: the
// system's own tool on the same pty, and the same tab in every other respect.
// `fresh` opens another session even with one open already: a second shell beside the first.
async function openSession(
  name,
  task = null,
  bcast = null,
  waiting = false,
  via = "ssh",
  fresh = false,
) {
  // A broadcast pane never counts as "already open": two grids are two grids.
  const key = bcast
    ? `bcast:${bcast.gid}:${name}`
    : via === "ssh"
      ? `term:${task ?? ""}:${name}`
      : `${via}:${name}`;
  if (!bcast && !fresh && showOpen(key)) return;
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
    via,
    kind: "term",
    key,
    term,
    fit,
    search,
    host,
    dead: false,
    since: Date.now(),
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
      if (s.wait) {
        // A focus or mouse report the far end left switched on is not a key.
        if (d.startsWith("\x1b[")) return;
        clearInterval(s.wait);
        s.wait = null;
        term.write("\x1b[2m── stopped waiting ──\x1b[0m\r\n");
        return;
      }
      // A cable has nothing to wait on: it is plugged in or it isn't.
      if (d === "r" && !task && via !== "serial") return waitForIt(s);
      const again = d === "\r" ? task : d === "c" && task === "ping" ? "ping-on" : undefined;
      if (again !== undefined) redial(s, again, false);
      return;
    }
    // Broadcast on: every live sibling gets the keystroke. Off: only the focused pane,
    // and a snippet filled in per pane is already in each of them.
    if (s.bcast?.on && !soloPaste) {
      for (const other of s.bcast.live) {
        invoke("write_session", { id: other, data: d }).catch(() => {});
      }
      return;
    }
    invoke("write_session", { id, data: d }).catch(() => {});
  });
  term.onResize(({ cols, rows }) => invoke("resize_session", { id, cols, rows }).catch(() => {}));
  term.onBell(() => heads(s, "rang its bell"));

  // Our banner is cleared by the first byte from the far end, so a login that draws
  // with cursor moves gets a clean screen. An error we wrote on the way in stays.
  let ours = true;
  const what = task ? `${taskLabel(task)} ` : via === "ssh" ? "" : `${via} `;
  term.write(`\x1b[2m── ${what}${name}… ──\x1b[0m\r\n`);

  const out = ptyChannel(
    s,
    (bytes) => {
      if (ours) {
        ours = false;
        term.write("\x1b[2J\x1b[3J\x1b[H");
      }
      term.write(bytes);
      if (via === "ssh") offerHostKey(s, bytes);
      // Output nobody is looking at: the tab says so until it is looked at. Only on the
      // change, so a busy build doesn't redraw the strip for every line; and only once the
      // tab has been in front, so tabs reopened at launch don't all light up with logins.
      if (!s.news && s.seen && !inActive(s)) {
        s.news = true;
        renderTabs();
      }
    },
    (code) => {
      s.dead = true;
      s.bcast?.live.delete(id);
      // 255 is ssh failing to get there or losing the box: the redial was too early.
      if (waiting && code === 255) waitForIt(s);
      else
        term.write(
          `\r\n\x1b[2m── ${task ? taskLabel(task) : via} exited (${code}) · ⏎ to ${task ? "run it again" : "reconnect"}${task === "ping" ? " · c to ping until ^C" : task || via === "serial" ? "" : " · r to reconnect when it's back"} · ${chord("w")} to close ──\x1b[0m\r\n`,
        );
      if (!task && !(waiting && code === 255)) lostCard(s, code);
      if (!task) heads(s, "disconnected");
      renderTabs();
      renderTree();
    },
  );

  try {
    const size = { cols: term.cols, rows: term.rows };
    await (task
      ? invoke("open_task", { out, id, name, task, ...size })
      : via === "ssh"
        ? invoke("open_session", { out, id, name, ...size })
        : invoke("open_line", { out, id, name, via, ...size }));
  } catch (err) {
    ours = false;
    s.dead = true;
    term.write(`\r\n\x1b[31m${String(err)}\x1b[0m\r\n`);
  }
  renderTabs();
  term.focus();
}

// A pty's raw bytes, then its exit code as a number: one channel, so the code can't
// overtake the last of the output. Bytes because a UTF-8 sequence can straddle two
// messages and only xterm's decoder sees both halves. An array instead of an
// ArrayBuffer is Tauri's postMessage fallback; see the CSP note in CLAUDE.md. Muted
// when the tab goes, since output keeps coming until the far end hangs up.
function ptyChannel(s, onBytes, onExit) {
  const chan = new window.__TAURI__.core.Channel();
  chan.onmessage = (m) => (typeof m === "number" ? onExit(m) : onBytes(new Uint8Array(m)));
  s.unlisten.push(() => (chan.onmessage = () => {}));
  return chan;
}

// The same device again in place of a dead tab: fresh, so a second shell of it alive
// beside this one isn't taken for it, and back into the split it sat in.
async function redial(s, task, waiting) {
  const g = s.split;
  const i = g ? g.ids.indexOf(s.id) : -1;
  const beside = g && (g.ids[i - 1] ?? g.ids[i + 1]);
  const dir = g?.dir;
  dropTab(s.id);
  await openSession(s.name, task, s.bcast, waiting, s.via, true);
  if (beside !== undefined && sessions.has(beside) && activeId !== beside) {
    joinSplit(beside, activeId, dir);
  }
}

// A reboot: dial again once the device takes a connection. Any key stops it.
function waitForIt(s) {
  s.lost?.remove();
  s.term.write(`\r\n\x1b[2m── waiting for ${labelOf(s.name)} · any key to stop ──\x1b[0m\r\n`);
  let asking = false;
  s.wait = setInterval(async () => {
    if (asking) return;
    asking = true;
    const up = await invoke("reachable", { name: s.name }).catch(() => false);
    asking = false;
    if (!up || !s.wait || !sessions.has(s.id)) return;
    redial(s, null, true);
  }, 3000);
}

// A shell that ended gets a card over its dimmed screen, saying so and offering the way
// back. The line under the scrollback says the same in keys, and they still work: the card
// never takes focus, so the dead tab keeps the keyboard and ⏎ and r mean what they did.
function lostCard(s, code) {
  const mins = Math.round((Date.now() - s.since) / 60000);
  const who = esc(labelOf(s.name));
  // ssh's 255 is "couldn't get there" as well as "lost it": the time it ran tells them apart
  // well enough to say which, and a session that never got anywhere has no "after".
  const lost = s.via === "ssh" && code === 255;
  const [title, said] =
    lost && mins < 1
      ? [`Couldn't reach ${who}`, "It didn't answer. The scrollback is kept."]
      : [
          lost ? `Connection to ${who} lost` : `${who} ended`,
          `${
            mins < 1
              ? "After less than a minute"
              : `After ${mins < 60 ? `${mins} min` : `${Math.floor(mins / 60)} h ${mins % 60} min`}`
          }. The scrollback is kept.`,
        ];
  const card = document.createElement("div");
  card.className = "lost";
  card.innerHTML = `<div class="lost-card" role="status">
    <span class="lost-i">${icon("unplug")}</span>
    <b>${title}</b>
    <span>${said}</span>
    <div class="lost-b">
      <button type="button" class="primary" data-lost="again">Reconnect<kbd>⏎</kbd></button>
      ${
        s.via === "serial"
          ? ""
          : `<button type="button" class="ghost" data-lost="wait" data-tip="Dial again once it answers">Wait and retry<kbd>r</kbd></button>`
      }
      <button type="button" class="ghost" data-lost="close">Close</button>
    </div>
  </div>`;
  card.addEventListener("mousedown", (e) => e.preventDefault());
  card.addEventListener("click", (e) => {
    const act = e.target.closest("[data-lost]")?.dataset.lost;
    if (act === "again") redial(s, null, false);
    if (act === "wait") waitForIt(s);
    if (act === "close") closeSession(s.id);
  });
  s.host.append(card);
  s.lost = card;
}

// A session elsewhere has something to say: a pill with the way to it, and a bounce of the
// Dock icon when the window isn't in front. Nothing for the session you are looking at, in
// a window you are looking at - the screen already said it.
function heads(s, what) {
  if (inActive(s) && document.hasFocus()) return;
  // A shell can ring its bell on every keystroke of a mistyped command: one notice a
  // session in any ten seconds, so the foot of the window and the Dock stay readable.
  if (Date.now() - (s.toldAt ?? 0) < 10000) return;
  s.toldAt = Date.now();
  flash(`${labelOf(s.name)} ${what}.`, false, {
    label: "Show",
    run: () => {
      if (!sessions.has(s.id)) return;
      activeId = s.id;
      showTab();
      renderTabs();
    },
  });
  invoke("attention").catch(() => {});
}

// A live tab: a connected shell or desktop. Closing one is not asked about - it is
// one deliberate click on one session - but closing several at once is.
const isLive = (s) =>
  !s.dead && ((s.kind === "term" && !s.task) || s.kind === "rdp" || s.kind === "vnc");
const liveSessions = () => [...sessions.values()].filter(isLive);

function closeSession(id) {
  const s = sessions.get(id);
  if (!s) return;
  const closer =
    { rdp: "close_rdp_session", vnc: "close_vnc_session", web: "close_web_view" }[s.kind] ??
    "close_session";
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
  // Before it goes, so the pane that inherits the keyboard is one beside it.
  const ids = s.split?.ids;
  const beside = ids && (ids[ids.indexOf(id) - 1] ?? ids[ids.indexOf(id) + 1]);
  leaveSplit(s);
  sessions.delete(id);
  s.bcast?.live.delete(id);
  if (activeId === id) {
    // Stay inside the same broadcast group or split when one of its panes closes.
    const sibling =
      (s.bcast && [...sessions.values()].find((x) => x.bcast === s.bcast)) ||
      (beside !== undefined && sessions.get(beside));
    activeId = sibling ? sibling.id : ([...sessions.keys()].pop() ?? null);
  }
  showTab();
  renderTabs();
  renderTree();
}

// activeId === null is the tree's own view, the detail of what it has selected;
// anything else is a session.
function showTab() {
  // The find bar searches one session's scrollback, so it does not follow you.
  if (!findEl.hidden) closeFind();
  browseEl.hidden = activeId !== null;
  termsEl.hidden = activeId === null;
  const grid = sessions.get(activeId)?.bcast ?? null;
  const split = sessions.get(activeId)?.split ?? null;
  if (split) split.last = activeId;
  termsEl.classList.toggle("grid", grid !== null);
  termsEl.classList.toggle("split", split !== null);
  termsEl.classList.toggle("col", split?.dir === "col");
  // Panes take their place by `order`, dividers sit between them, and a pane's share
  // of the room is its flex-grow.
  for (const d of termsEl.querySelectorAll(".divider")) d.remove();
  if (split) {
    split.ids.forEach((id, i) => {
      if (!i) return;
      const d = document.createElement("div");
      d.className = "divider";
      d.dataset.at = i;
      d.setAttribute("role", "separator");
      d.setAttribute("aria-orientation", split.dir === "col" ? "horizontal" : "vertical");
      d.style.order = 2 * i - 1;
      termsEl.append(d);
    });
  }
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
    s.host.classList.toggle("focused", (grid !== null || split !== null) && s.id === activeId);
    const at = split ? split.ids.indexOf(s.id) : -1;
    s.host.style.order = at < 0 ? "" : 2 * at;
    s.host.style.flexGrow = at < 0 ? "" : split.sizes[at];
    if (at >= 0) s.host.dataset.name = labelOf(s.name);
    else if (!s.bcast) delete s.host.dataset.name;
  }
  if (activeId !== null) {
    // Every visible pane refits after the grid template lands.
    requestAnimationFrame(() => {
      if (grid || split) {
        for (const s of sessions.values()) if (inActive(s)) s.fit?.fit();
      }
      const s = sessions.get(activeId);
      s?.fit?.fit();
      // A prompt opened since (a sign-in asked again after a tab closed) keeps its field.
      if (!modalOpen()) (s?.term ?? s?.canvas)?.focus();
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

// ssh says this once, on the first connect to a host it didn't know, and for a team
// device that is the moment its key is worth sharing: the button in the detail is
// where few look. Asked of the files rather than trusted from the text, which could
// be anything the far end printed, or a bastion on the way being added.
// Not settled by the first line: through a jump host, that one is the bastion's.
// Decoded per chunk, not as a stream: a character split across two only spoils
// itself, and the words looked for are ASCII.
const utf8 = new TextDecoder();
function offerHostKey(s, bytes) {
  if (s.keyDone || s.keyAsking || s.task) return;
  const j = all.find((x) => x.name === s.name);
  if (j?.vault !== "team" || !utf8.decode(bytes).includes("Permanently added")) return;
  s.keyAsking = true;
  invoke("host_key", { name: j.name })
    .then((k) => {
      if (!k.mine.length) return;
      s.keyDone = true;
      if (k.team.length) return;
      flash(`First connect to "${j.label}". Share its host key with the team?`, false, {
        label: "Share",
        run: () => shareHostKey(j),
      });
    })
    .catch(() => {})
    .finally(() => (s.keyAsking = false));
}
async function shareHostKey(j) {
  try {
    await invoke("share_host_key", { name: j.name });
    hostKeys.delete(j.name);
    flash(`The team now checks "${j.label}" against the key this machine has.`);
  } catch (err) {
    alertish(err);
  }
  renderDetail();
}

// A live shell is the tab in front: the only place a snippet has a prompt to go to.
function shellInFront() {
  const s = sessions.get(activeId);
  return !!s && s.kind === "term" && !s.task && !s.dead;
}

// A snippet goes in as a paste, which xterm hands to the same `onData` a keystroke
// takes: so broadcast fans it out, and a shell with bracketed paste on shows it as
// pasted text. It is never followed by Enter. It stops at the shell's screen like
// everything else that sends bytes: in an editor the text would be commands, and in
// a broadcast one pane in an editor stops it for all of them.
//
// `{host}`, `{user}`, `{port}` and `{name}` are the device's own, so one snippet serves
// every machine, and in a broadcast each pane gets its own. Filled in before the check,
// not after: a host is a colleague's text too, and must not be the thing that presses
// Enter.
//
// `{?}` is a blank, asked for when the snippet is picked, once for every pane of a
// broadcast; `{?unit}` says what goes there. Asked in order, and a cancel types nothing.
let soloPaste = false; // set while each broadcast pane takes its own copy
function fillSnippet(text, name) {
  const j = all.find((x) => x.name === name);
  const said = {
    host: j?.host,
    user: j?.user,
    port: String(j?.port ?? 22),
    name: j?.label ?? name,
  };
  const missing = [];
  const out = text.replace(/\{(host|user|port|name)\}/g, (m, k) => {
    if (said[k]) return said[k];
    missing.push(k);
    return m;
  });
  if (missing.length) throw `"${labelOf(name)}" has no ${missing[0]} to fill in`;
  return out;
}
async function typeSnippet(sn) {
  if (!shellInFront()) return alertish("a snippet needs a shell in front to be typed into");
  // Asked before the panes are looked at: the far end can open an editor, or a pane
  // close, while the question is up. One pass, so an answer with a {?} in it stays text.
  const answers = [];
  for (const [, what] of sn.text.matchAll(/\{\?([^}]*)\}/g)) {
    const v = await ask(`${sn.name}: ${what.trim() || "fill in the blank"}`, "", "Type");
    if (v === null) return sessions.get(activeId)?.term.focus();
    answers.push(v);
  }
  const text = sn.text.replace(/\{\?([^}]*)\}/g, () => answers.shift());
  const s = sessions.get(activeId);
  if (!shellInFront()) return alertish("a snippet needs a shell in front to be typed into");
  const panes = s.bcast?.on ? [...s.bcast.live].map((id) => sessions.get(id)).filter(Boolean) : [s];
  const busy = panes.find((p) => p.term.buffer.active.type !== "normal");
  if (busy) {
    return alertish(
      `nothing was typed: "${labelOf(busy.name)}" is in an editor, where the text would be commands`,
    );
  }
  let filled;
  try {
    filled = panes.map((p) => fillSnippet(text, p.name));
  } catch (e) {
    return alertish(`nothing was typed: ${e}`);
  }
  if (!filled.every(snippetOk)) {
    return alertish(`"${sn.name}" isn't one plain line, so nothing was typed`);
  }
  // xterm's paste fires `onData` before it returns, so the flag covers exactly these.
  soloPaste = true;
  try {
    panes.forEach((p, i) => p.term.paste(filled[i]));
  } finally {
    soloPaste = false;
  }
  s.term.focus();
  flash(`Typed "${sn.name}". Enter runs it.`);
}

// The toolbar's way in, for whoever never finds them in Search: every snippet, and
// the way to write one.
function snippetMenu(btn) {
  const r = btn.getBoundingClientRect();
  showCtx(r.left, r.bottom + 4, null, [
    ...snips.map((sn) => ({
      icon: "text-cursor-input",
      label: sn.name,
      run: () => typeSnippet(sn),
    })),
    ...(snips.length ? ["-"] : []),
    {
      icon: snips.length ? "pencil" : "plus",
      label: snips.length ? "Edit snippets…" : "Add a snippet…",
      run: () => openSettings("snippets"),
    },
  ]);
}

// Whether something of ours has to be drawn where the page is. Sheets and the palette
// cover the window; a menu only counts when it actually reaches over the page, so a
// right-click in the sidebar no longer blanks the tab beside it.
function covered(r) {
  return OVERLAYS().some((el) => {
    if (el.hidden) return false;
    const o = el.getBoundingClientRect();
    return o.left < r.right && o.right > r.left && o.top < r.bottom && o.bottom > r.top;
  });
}

function placeWebViews() {
  for (const s of sessions.values()) {
    if (s.kind !== "web") continue;
    const r = s.host.getBoundingClientRect();
    // Held back until the first page finishes: before that WebKit paints a white
    // pane, and a NAS waking its disks can sit on one for half a minute.
    const shown = inActive(s) && !covered(r) && !s.failed && s.loaded && r.width > 0;
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
  if (s.loaded && !s.failed) return;
  s.loaded = true;
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

// "EMAIL=it@x,CN=nas,OU=IT,..." reads as "nas"; a certificate without one reads whole.
const commonName = (dn) => /(?:^|,)\s*CN=([^,]+)/.exec(dn)?.[1] ?? dn;

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
          `today - trusting it covers the certificates it issues for this device.`
        : null;
      const ends = new Date(c.expires);
      const expired = ends < new Date();
      const ok = await askFacts(
        `Trust ${commonName(c.subject)}?`,
        scope,
        [
          ["Issued by", commonName(c.issuer)],
          [
            expired ? "Expired" : "Expires",
            isNaN(ends) ? c.expires : ends.toLocaleDateString(),
            expired ? "bad" : "",
          ],
          ["SHA-256", c.fingerprint, "mono"],
        ],
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

  const out = ptyChannel(
    s,
    (bytes) => term.write(bytes),
    async () => {
      if (s.master !== id || s.entries) return;
      // ControlPersist backgrounds the client once it has authenticated, so the
      // socket, not the exit, says whether the sign-in worked.
      await new Promise((r) => setTimeout(r, 400));
      if (await invoke("sftp_ready", { name: s.name }).catch(() => false)) return connected(s);
      failedSignIn(s, "that sign-in didn't finish");
    },
  );
  try {
    term.write(`\x1b[2m── signing in to ${s.name} for files ──\x1b[0m\r\n`);
    await invoke("open_master", { out, id, name: s.name, cols: term.cols, rows: term.rows });
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

  const s = { id, name, kind: "web", key, host, dead: false, loaded: false, unlisten: [] };
  host.innerHTML = `<div class="webwait">${icon("globe")}<b>${esc(labelOf(name))}</b>
    <span>${esc(key.slice(`web:${name}:`.length))}</span></div>`;
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();

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
    // Before the view exists: a page quicker than the round trip back from
    // `open_web_view` would otherwise finish unheard and stay behind the note.
    s.unlisten.push(await listen(`web-load:${id}`, () => webLoaded(s)));
  } catch {
    // No capability means no redirect notice and no load notice, not a broken tab:
    // show the page now rather than wait for an event that can't arrive.
    s.loaded = true;
    s.host.innerHTML = "";
  }

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

  armWebCheck(s, url);
}

function renderTabs() {
  // A broadcast group is one chip, selected whenever any of its panes has focus; so is a
  // split, named after its panes.
  const seen = new Set();
  const chips = [];
  // Whatever is in front has been seen.
  for (const s of sessions.values()) {
    if (inActive(s)) {
      s.news = false;
      s.seen = true;
    }
  }
  for (const s of sessions.values()) {
    if (s.split) {
      if (seen.has(`s${s.split.sid}`)) continue;
      seen.add(`s${s.split.sid}`);
      const g = s.split;
      const panes = g.ids.map((id) => sessions.get(id));
      const names = panes.map((p) => labelOf(p.name)).join(" · ");
      chips.push(`<div class="tab split ${panes.every((p) => p.dead) ? "dead" : ""} ${panes.some((p) => p.news) ? "news" : ""}" data-sid="${g.sid}" role="tab" aria-selected="${g.ids.includes(activeId)}">
        <span class="tabkind">${icon(g.dir === "col" ? "rows-2" : "columns-2")}</span>
        <span class="lbl">${esc(names)}</span>
        <span class="x" data-sclose="${g.sid}" data-tip="Close all" role="button">${icon("x")}</span>
      </div>`);
      continue;
    }
    if (s.bcast) {
      if (seen.has(s.bcast.gid)) continue;
      seen.add(s.bcast.gid);
      const b = s.bcast;
      const anyActive = [...sessions.values()].some((x) => x.bcast === b && x.id === activeId);
      const alive = b.live.size;
      chips.push(`<div class="tab bcast ${b.on ? "on" : ""}" data-gid="${b.gid}" role="tab" aria-selected="${anyActive}">
        <span class="tabkind">${icon("radio-tower")}</span>
        <span class="lbl">Broadcast · ${alive} of ${b.names.length}</span>
        <button type="button" class="bmute" data-bmute="${b.gid}"
          data-tip="${b.on ? "Stop broadcasting keystrokes" : "Broadcast keystrokes to every pane"}"
          aria-pressed="${b.on}">${icon(b.on ? "radio-tower" : "square")}</button>
        <span class="x" data-bclose="${b.gid}" data-tip="Close all" role="button">${icon("x")}</span>
      </div>`);
      continue;
    }
    // A shell tab wears its device's mark in readable()'s colour, never a raw brand hex.
    const os = s.kind === "term" && !s.task ? all.find((j) => j.name === s.name)?.os : undefined;
    const tint = os && osColor(os);
    chips.push(`<div class="tab ${s.dead ? "dead" : ""} ${s.news ? "news" : ""}" data-id="${s.id}" role="tab" aria-selected="${s.id === activeId}">
      <span class="dot ${s.dead ? "down" : "up"}" role="img" aria-label="${s.dead ? "ended" : s.news ? "connected, new output" : "connected"}"></span>
      ${os ? `<span class="tabkind"${tint ? ` style="color:${esc(tint)}"` : ""}>${osIcon(os)}</span>` : ""}
      ${s.kind === "rdp" ? `<span class="tabkind">${icon("monitor")}</span>` : ""}
      ${s.kind === "vnc" ? `<span class="tabkind">${icon("screen-share")}</span>` : ""}
      ${s.kind === "web" ? `<span class="tabkind">${icon("globe")}</span>` : ""}
      ${s.kind === "sftp" ? `<span class="tabkind">${icon("folder")}</span>` : ""}
      ${s.task ? `<span class="tabkind">${icon({ trace: "waypoints", key: "key-round" }[s.task] ?? "plug")}</span>` : ""}
      <span class="lbl">${esc(s.task ? `${taskLabel(s.task)} ${labelOf(s.name)}` : labelOf(s.name))}</span>
      ${
        s.kind === "web" && webextRunning && !s.dead
          ? `<span class="x ${s.filling ? "busy" : ""}" data-bw="${s.id}" data-tip="Fill · right-click for Bitwarden" role="button">${icon("key-round")}</span>`
          : ""
      }
      <span class="x" data-close="${s.id}" data-tip="Close  ${chord("w")}" role="button">${icon("x")}</span>
    </div>`);
  }
  tabsEl.innerHTML = chips.join("");
  // Here rather than in render(): the extension starts after the first paint, and
  // which chips apply changes with the tab in front, which is not a render.
  const live = sessions.get(activeId);
  const vault = $("vault");
  vault.hidden = !webextRunning;
  vault.innerHTML = `${icon("key-round")}Vault`;
  vault.dataset.tip = "Open Bitwarden";
  // The map is of the tree's selection; over a session it would be a button for a
  // view that is not on screen. The pane beside a session is the other way round.
  $("viewmode").hidden = !!live;
  const snipBtn = $("snipbtn");
  snipBtn.hidden = !shellInFront();
  snipBtn.innerHTML = `${icon("text-cursor-input")}Snippets`;
  snipBtn.dataset.tip = "Type a snippet at this prompt";
  const details = $("details");
  details.hidden = !live;
  details.innerHTML = `${icon("panel-right")}Details`;
  details.setAttribute("aria-pressed", detailsOpen);
  details.dataset.tip = detailsOpen ? "Hide the device's details" : "Show the device's details";
  appEl.classList.toggle("details", !!live && detailsOpen);
  if (live && detailsOpen) renderDetail();
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
  tellPresence();
}

// Which of the team's devices this window has a session on, told to Rust whenever it
// changes. Rust keeps the file beside the shared list and its heartbeat: a window in
// the background has its timers slowed, and a session is just as open back there.
function tellPresence() {
  const names = vaults.two
    ? [...new Set([...sessions.values()].filter((s) => !s.dead).map((s) => s.name))]
        .filter((n) => all.find((j) => j.name === n)?.vault === "team")
        .sort()
    : [];
  const said = names.join("\n");
  if (said === presentSaid) return;
  presentSaid = said;
  invoke("presence_set", { names }).catch(() => {});
}

// Shells, pages and file listings come back on the next launch. Stored in localStorage
// like `recent`: a habit, not part of the list.
function rememberTabs() {
  const open = [...sessions.values()]
    .filter((s) => !s.dead && !s.bcast && !s.task && s.kind !== "rdp" && s.kind !== "vnc")
    .map((s) => ({
      kind: s.kind,
      name: s.name,
      ...(s.via !== "ssh" && s.via ? { via: s.via } : {}),
    }));
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
  // A fill answers once it has landed, which can be seconds while Bitwarden's worker
  // wakes, so the key pulses until then.
  const s = popup ? undefined : sessions.get(id);
  // A count, so a second click doesn't stop the pulse when the first fill lands.
  if (s) {
    s.filling = (s.filling ?? 0) + 1;
    renderTabs();
  }
  invoke("webext_key", { id, ...at, popup })
    .catch(alertish)
    .finally(() => {
      if (!s) return;
      s.filling--;
      renderTabs();
    });
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
  const sclose = e.target.closest("[data-sclose]")?.dataset.sclose;
  if (sclose) return closeSplit(+sclose);
  const split = e.target.closest("[data-sid]");
  if (split) {
    // Back to the pane that last had the keyboard.
    const g = [...sessions.values()].find((s) => s.split?.sid === +split.dataset.sid)?.split;
    if (g) {
      activeId = g.last;
      showTab();
      renderTabs();
    }
    return;
  }
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
  activeId = +tab.dataset.id;
  showTab();
  renderTabs();
});

// Drag a tab along the strip to reorder it. Pointer events for dragFrom's reason. The
// chip moves in the DOM as it goes and the drop rewrites `sessions` in that order,
// which is the order every render, ⌘[ / ⌘] and the reopened tabs read.
let tabDragged = false; // the click that ends a drag is not a click
tabsEl.addEventListener("pointerdown", (e) => {
  const chip = e.target.closest(".tab");
  if (!chip || e.button !== 0 || e.target.closest(".x, button")) return;
  const x0 = e.clientX;
  let lifted = false;
  const onMove = (ev) => {
    if (!lifted) {
      if (Math.abs(ev.clientX - x0) < 6) return;
      lifted = true;
      tabsEl.setPointerCapture(e.pointerId);
      document.body.classList.add("dragging");
      chip.classList.add("lifted");
    }
    // The strip's middle, so a pointer drifting up or down still finds a tab.
    const r = tabsEl.getBoundingClientRect();
    const over = document.elementFromPoint(ev.clientX, r.top + r.height / 2)?.closest(".tab");
    if (!over || over === chip) return;
    const b = over.getBoundingClientRect();
    if (ev.clientX < b.left + b.width / 2) over.before(chip);
    else over.after(chip);
  };
  const end = () => {
    tabsEl.removeEventListener("pointermove", onMove);
    tabsEl.removeEventListener("pointerup", end);
    tabsEl.removeEventListener("lostpointercapture", end);
    if (!lifted) return;
    document.body.classList.remove("dragging");
    tabDragged = true;
    setTimeout(() => (tabDragged = false), 0);
    const was = [...sessions.values()];
    const order = [...tabsEl.querySelectorAll(".tab")].flatMap((t) =>
      was.filter((s) =>
        t.dataset.gid
          ? s.bcast?.gid === +t.dataset.gid
          : t.dataset.sid
            ? s.split?.sid === +t.dataset.sid
            : s.id === +t.dataset.id,
      ),
    );
    // A session opened mid-drag has no chip in the order yet; it goes last.
    for (const s of was) if (!order.includes(s)) order.push(s);
    sessions.clear();
    for (const s of order) sessions.set(s.id, s);
    renderTabs();
  };
  tabsEl.addEventListener("pointermove", onMove);
  tabsEl.addEventListener("pointerup", end);
  tabsEl.addEventListener("lostpointercapture", end);
});
tabsEl.addEventListener(
  "click",
  (e) => {
    if (!tabDragged) return;
    tabDragged = false;
    e.stopImmediatePropagation();
  },
  true,
);

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
  // One step per chip: a broadcast group is one tab in the strip. A split's panes are a
  // step each, so the keyboard can reach every one of them.
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

// Typed sign-ins for the window's lifetime. A remembered one is only ever `password:
// null` here: the keychain's copy stays in Rust and never crosses into the page.
const rdpCreds = new Map();

// The prompt, offering every account remembered on this machine and starting on the
// last one saved, so Enter is all a single saved sign-in costs. The Remember box is only
// there when there is a keychain to keep it in.
async function rdpSaved(name, j) {
  const saved = await invoke("rdp_saved", { name }).catch(() => ({ users: [], can: false }));
  return rdpAsk(j, saved.users[0] ?? j.user ?? "", saved.can ? false : null, saved.users);
}

// The username is asked for even with one in the config: a Windows box is usually
// reached as a different account than ssh uses.
const rdpAsk = (j, user, remember, saved = []) =>
  ask(`Sign in to ${j.host}`, "", "Connect", "password", user, remember, saved);

// One remote desktop tab, RDP or VNC: a canvas Rust paints tiles onto, and the mouse and
// keyboard going back. Both protocols send the same records, and `${kind}_input` takes
// the same arguments, so only opening one differs. The tab shows before the server
// answers, so connecting has somewhere to be.
function desktopTab(name, kind) {
  const key = `${kind}:${name}`;
  // A dead desktop is a black canvas with nothing to read back, so a retry takes its
  // place rather than lining up beside it.
  for (const d of sessions.values()) if (d.key === key && d.dead) dropTab(d.id);

  const id = nextId++;
  const host = document.createElement("div");
  host.className = "termhost rdphost";
  const canvas = document.createElement("canvas");
  canvas.tabIndex = 0; // so it can take the keyboard at all
  host.append(canvas);
  termsEl.append(host);
  const ctx = canvas.getContext("2d");
  watchDrops();

  const s = { id, name, kind, key, canvas, host, dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();

  // `key` is the character a keystroke typed: VNC sends characters, RDP ignores it.
  const send = (k, a = 0, b = 0, down = false, extra = {}) =>
    invoke(`${kind}_input`, { id, kind: k, a, b, down, ...extra }).catch(() => {});

  // The desktop is letterboxed into the pane while it is a different size: an RDP resize
  // is a round trip to the server, and a VNC desktop is never resized at all. The
  // element keeps the picture's aspect so `at()` stays a plain rect ratio.
  const scale = () => {
    // A backgrounded tab has no box, and a canvas sized to nothing would stay that
    // way if the observer didn't fire again on the way back.
    if (!host.clientWidth || !host.clientHeight) return;
    const k = Math.min(host.clientWidth / canvas.width, host.clientHeight / canvas.height);
    canvas.style.width = `${Math.round(canvas.width * k)}px`;
    canvas.style.height = `${Math.round(canvas.height * k)}px`;
    if (shape) cursor(...shape);
  };
  const ro = new ResizeObserver(scale);
  ro.observe(host);
  s.unlisten.push(() => ro.disconnect());

  // The server's pointer shape on the canvas's own cursor, so it moves without a round
  // trip. It comes in desktop pixels, and the desktop is drawn smaller than that (a 2x
  // screen's RDP desktop is in device pixels, a VNC one letterboxed), so it is scaled the
  // same way or Windows' I-beam is twice the size of the text it sits on. Kept, so a
  // pane that changes size redraws it. Empty is the server hiding it; a hotspot x of
  // 0xffff with no pixels is the system arrow.
  let shape = null;
  const cursor = (w, h, hx, hy, rgba) => {
    shape = [w, h, hx, hy, rgba];
    if (!rgba) return void (canvas.style.cursor = hx === 0xffff ? "" : "none");
    const k = parseFloat(canvas.style.width) / canvas.width || 1;
    const src = document.createElement("canvas");
    src.width = w;
    src.height = h;
    src.getContext("2d").putImageData(new ImageData(rgba, w, h), 0, 0);
    const c = document.createElement("canvas");
    c.width = Math.max(1, Math.round(w * k));
    c.height = Math.max(1, Math.round(h * k));
    c.getContext("2d").drawImage(src, 0, 0, c.width, c.height);
    canvas.style.cursor = `url(${c.toDataURL()}) ${Math.round(hx * k)} ${Math.round(hy * k)}, default`;
  };

  // A remote desktop's sound, played as it arrives: each block scheduled right after the
  // last, and after a gap (silence, a stall) a little ahead of the clock so the first
  // block of the next sound doesn't start late and click. A backlog past half a second
  // is dropped, so sound never falls behind the picture. Only RDP sends any; the context
  // is made on the first block, so a silent desktop never opens one.
  let audio = null,
    playAt = 0;
  const sound = new window.__TAURI__.core.Channel();
  sound.onmessage = (msg) => {
    const buf = msg instanceof ArrayBuffer ? msg : new Uint8Array(msg).buffer;
    const view = new DataView(buf);
    const rate = view.getUint32(0, true),
      channels = view.getUint16(4, true);
    const pcm = new Int16Array(buf, 8, (buf.byteLength - 8) >> 1);
    const frames = Math.floor(pcm.length / channels);
    if (!frames) return;
    audio ??= new AudioContext();
    const block = audio.createBuffer(channels, frames, rate);
    for (let c = 0; c < channels; c++) {
      const out = block.getChannelData(c);
      for (let i = 0; i < frames; i++) out[i] = pcm[i * channels + c] / 32768;
    }
    const src = audio.createBufferSource();
    src.buffer = block;
    src.connect(audio.destination);
    const now = audio.currentTime;
    if (playAt < now || playAt > now + 0.5) playAt = now + 0.05;
    src.start(playAt);
    playAt += block.duration;
  };
  // A webview may hold a context made outside a click until there is one; a click in
  // the desktop is one.
  canvas.addEventListener("mousedown", () => audio?.resume());
  s.unlisten.push(() => audio?.close());

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
      // x at 0xfffe is VNC's cursor: its hotspot as two more u16s, then its pixels.
      if (x === 0xfffe) {
        const hx = view.getUint16(at, true),
          hy = view.getUint16(at + 2, true);
        at += 4;
        cursor(w, h, hx, hy, w && h ? new Uint8ClampedArray(buf, at, w * h * 4) : null);
        at += w * h * 4;
        continue;
      }
      ctx.putImageData(new ImageData(new Uint8ClampedArray(buf, at, w * h * 4), w, h), x, y);
      at += w * h * 4;
    }
  };
  // The session holds the next frame until this one is painted, so a window that
  // paints slower than the desktop changes skips pictures rather than falling behind.
  const painted = () => send("painted");
  const chan = new window.__TAURI__.core.Channel();
  chan.onmessage = (msg) => {
    const buf = msg instanceof ArrayBuffer ? msg : new Uint8Array(msg).buffer;
    if (queued) return queued.push(buf);
    try {
      paint(buf);
    } finally {
      painted();
    }
  };

  // The size the server gave, once the open command resolves; what was held is painted.
  const show = (screen) => {
    canvas.width = screen.width;
    canvas.height = screen.height;
    scale();
    const held = queued;
    queued = null;
    try {
      held.forEach(paint);
    } finally {
      if (held.length) painted();
    }
  };

  // Once the session is up: its hang-up, then the mouse and keyboard.
  const live = async () => {
    // Without this a hang-up freezes the picture while the tab keeps a live dot.
    try {
      s.unlisten.push(
        await listen(`${kind}-exit:${id}`, (e) => {
          s.dead = true;
          renderTabs();
          renderTree();
          if (e.payload) alertish(e.payload);
        }),
      );
    } catch {
      /* no capability means no exit notice, not a broken session */
    }

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
    // A trackpad fires a wheel event per frame with a few pixels in each, and sending
    // each as a whole notch scrolled a page per flick and had the far end repaint sixty
    // times a second. The distance is summed and sent once a frame instead, in Windows'
    // units: 120 is a notch, three lines, which is about 60 of our pixels.
    // ponytail: a fixed pixels-to-units ratio; per-app tuning if a program scrolls oddly.
    let wheel = 0,
      wheelFrame = 0;
    const scroll = () => {
      // Two notches a frame at most, which is all the wire's nine bits hold; the rest
      // stays in `wheel` for the next frame rather than wrapping into the other way.
      const units = Math.max(-240, Math.min(240, Math.round(wheel * -2)));
      wheel += units / 2;
      wheelFrame = Math.abs(wheel) >= 0.5 ? requestAnimationFrame(scroll) : 0;
      if (!units) return;
      // A wheel carries no coordinates either: it lands wherever the last move left it.
      flush();
      send("wheel", units);
    };
    canvas.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        wheel += e.deltaY * [1, 20, 400][e.deltaMode];
        wheelFrame ||= requestAnimationFrame(scroll);
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
    // ⌘C, ⌘X, ⌘V, ⌘A and ⌘Z are Ctrl on the far end, as in Microsoft's own Mac client.
    // ⌘ is already down there as Win by then, and Win+V is Windows' clipboard history.
    const ctrlChord = (code) => {
      send("key", SCANCODES.ControlLeft, 0, true);
      // Let go of with Ctrl already down, or Windows reads a lone Win and opens Start.
      for (const win of [SCANCODES.MetaLeft, SCANCODES.MetaRight])
        if (down.delete(win)) send("key", win, 0, false);
      send("key", code, 0, true);
      send("key", code, 0, false);
      send("key", SCANCODES.ControlLeft, 0, false);
    };
    const MAC_EDIT = ["KeyC", "KeyX", "KeyV", "KeyA", "KeyZ"];
    // When the Edit menu takes the chord it never arrives as a keydown, only as its
    // clipboard event; the `before` ones are what enable the menu item over a canvas.
    const EDIT = { copy: "KeyC", cut: "KeyX", paste: "KeyV" };
    const onEdit = (e) => {
      if (document.activeElement !== canvas) return;
      e.preventDefault();
      if (EDIT[e.type]) ctrlChord(SCANCODES[EDIT[e.type]]);
    };
    if (isMac) {
      const types = ["beforecopy", "beforecut", "beforepaste", ...Object.keys(EDIT)];
      for (const t of types) document.addEventListener(t, onEdit, true);
      s.unlisten.push(() => types.forEach((t) => document.removeEventListener(t, onEdit, true)));
    }
    for (const [type, pressed] of [
      ["keydown", true],
      ["keyup", false],
    ]) {
      canvas.addEventListener(type, (e) => {
        const code = SCANCODES[e.code];
        if (code === undefined) return;
        // Window chords stay ours; everything else belongs to the remote desktop.
        if (chorded(e) && ["w", "k", "n", "d", "[", "]"].includes(chordKey(e))) return;
        e.preventDefault();
        if (isMac && e.metaKey && !e.ctrlKey && MAC_EDIT.includes(e.code)) {
          if (pressed) ctrlChord(code);
          return;
        }
        if (pressed) down.add(code);
        else down.delete(code);
        // ⌥ on a Mac is Alt on the far end, not the character it would type here: ⌥F
        // is a menu there, not ƒ. Without a character VNC falls back to the key itself.
        send("key", code, 0, pressed, { key: isMac && e.altKey ? null : e.key });
        if (!pressed && (e.code === "MetaLeft" || e.code === "MetaRight")) release();
      });
    }
    canvas.addEventListener("blur", release);
    // VNC sends the local clipboard here and nowhere else; RDP ignores it.
    canvas.addEventListener("focus", () => send("focus"));

    renderTabs();
    canvas.focus();
  };

  return { s, send, chan, sound, show, live };
}

async function openRdpSession(name) {
  const j = all.find((x) => x.name === name);
  if (!j) return;
  // Before the credentials are asked for: a second desktop would be a second login.
  if (showOpen(`rdp:${name}`)) return;

  const creds = rdpCreds.get(name) ?? (await rdpSaved(name, j));
  if (!creds) return;
  const d = desktopTab(name, "rdp");
  const { id, host } = d.s;

  // The pane's own size in device pixels, so the desktop fits without being scaled:
  // in CSS pixels a 2x screen draws each one as a 2x2 block and the text goes soft.
  // `scale` is what keeps it from being half the size. Even numbers because the RDP
  // codecs work in 2x2 blocks. Laid out by showTab() in desktopTab.
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
      remember: creds.remember ?? null,
      width: asked[0],
      height: asked[1],
      scale: asked[2],
      onTile: d.chan,
      onSound: d.sound,
    });
    // The server picks the size; asking for one is only a suggestion. What it gave is
    // what the pane is compared against, or a desktop that came back smaller than the
    // pane is never asked again and sits letterboxed at the wrong scale.
    d.show(screen);
    asked = [screen.width, screen.height, asked[2]];
    // Every resize tears the session down and rebuilds it, so the pointer is only
    // believed once it has stopped moving. A server without the Display Control
    // channel ignores it and the letterboxing is all there is.
    let settle;
    const resized = () => {
      // A hidden pane measures zero, and asking for that would shrink the desktop to
      // the minimum every time another tab is looked at.
      if (!host.clientWidth || !host.clientHeight) return;
      const want = [px(host.clientWidth), px(host.clientHeight), percent()];
      if (want.every((n, i) => n === asked[i])) return;
      asked = want;
      clearTimeout(settle);
      settle = setTimeout(() => d.send("resize", want[0], want[1], false, { scale: want[2] }), 400);
    };
    const ro = new ResizeObserver(resized);
    ro.observe(host);
    // A move to a screen with another pixel ratio changes no box, so the observer says
    // nothing. The query names one ratio, so it is asked again each time it stops matching.
    let ratio;
    const moved = () => {
      ratio = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
      ratio.addEventListener("change", moved, { once: true });
      resized();
    };
    moved();
    d.s.unlisten.push(() => {
      ro.disconnect();
      ratio.removeEventListener("change", moved);
      clearTimeout(settle);
    });
    // Said once: a second connect this run neither saves it again nor forgets it.
    rdpCreds.set(name, { ...creds, remember: null });
  } catch (err) {
    d.s.dead = true;
    // Rejected credentials must not be remembered, or the next attempt reuses them.
    rdpCreds.delete(name);
    renderTabs();
    // A server too old for the built-in session (Server 2008 R2) still takes the
    // desktop's own client, so it goes there rather than ending at an error.
    if (String(err).includes("older encryption")) {
      dropTab(id);
      flash(`${j.name} only speaks older encryption, so it is opening in the Remote Desktop app`);
      return handOffRdp(name);
    }
    // A changed certificate is refused before the password is sent, so the sign-in
    // is kept for the retry. The fingerprint is the one the refusal named: trusting it
    // lets in that certificate and not whatever the host serves by the time we ask.
    const changed = String(err).match(/SHA-256 ([0-9a-f]{64})/);
    if (!changed && creds.password === null) {
      // A remembered password that didn't get in: asked for again, still ticked, so the
      // new one replaces it. Unticking forgets it instead.
      alertish(err);
      dropTab(id);
      const again = await rdpAsk(j, creds.user, true);
      if (!again) return;
      rdpCreds.set(name, again);
      return openRdpSession(name);
    }
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
  await d.live();
}

// Passwords typed this run, as `rdpCreds` keeps sign-ins. A remembered one never
// crosses into the page: `null` asks Rust to use it, and only if the server wants one.
const vncPasswords = new Map();

// Tried without asking first: most VNC servers worth a tab (a hypervisor console, a KVM)
// want no password, and the one that does says so.
async function openVncSession(name, password = vncPasswords.get(name) ?? null, remember = null) {
  const j = all.find((x) => x.name === name);
  if (!j) return;
  if (showOpen(`vnc:${name}`)) return;

  const d = desktopTab(name, "vnc");
  try {
    d.show(
      await invoke("open_vnc_session", { id: d.s.id, name, password, remember, onTile: d.chan }),
    );
    if (password !== null) vncPasswords.set(name, password);
  } catch (err) {
    dropTab(d.s.id);
    vncPasswords.delete(name);
    const e = String(err);
    // Apple's sign-in, VeNCrypt, RealVNC's: the system's viewer speaks what this can't.
    if (e.includes("signs in a way the built-in viewer")) {
      flash(
        `${j.name} signs in a way the built-in viewer doesn't speak, so it is opening in your VNC viewer`,
      );
      return handOffVnc(name);
    }
    const refused = e.includes("refused the sign-in");
    if (!refused && !e.includes("asks for a VNC password")) return alertish(err);
    if (refused) alertish(err);
    // A remembered password that was refused is asked for again still ticked, so the
    // new one replaces it.
    const can = await invoke("can_remember").catch(() => false);
    const typed = await ask(
      `VNC password for ${j.host}`,
      "",
      "Connect",
      "password",
      null,
      can ? refused && password === null : null,
    );
    if (!typed) return;
    return typeof typed === "string"
      ? openVncSession(name, typed)
      : openVncSession(name, typed.password, typed.remember);
  }
  await d.live();
}
