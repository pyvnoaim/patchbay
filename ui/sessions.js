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

  const s = { id, name, kind: "term", term, fit, host, dead: false, unlisten: [] };
  sessions.set(id, s);
  activeId = id;
  showTab();
  renderTabs();
  renderTree();
  fit.fit();

  term.onData((d) => invoke("write_session", { id, data: d }).catch(() => {}));
  term.onResize(({ cols, rows }) => invoke("resize_session", { id, cols, rows }).catch(() => {}));

  try {
    s.unlisten.push(await listen(`pty:${id}`, (e) => term.write(e.payload)));
    s.unlisten.push(await listen(`pty-exit:${id}`, (e) => {
      s.dead = true;
      term.write(`\r\n\x1b[2m── ssh exited (${e.payload}) · ⌘W to close ──\x1b[0m\r\n`);
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

  // Bring the folder's VPN up first, so connecting is one action, not two.
  const vpath = prefs.vpn_auto_connect !== false ? vpnFor(name) : null;
  if (vpath && !vpns.get(vpath)?.up) {
    term.write(`\x1b[2m── ${vpath} VPN is down, connecting… ──\x1b[0m\r\n`);
    try {
      await invoke("vpn_toggle", { path: vpath, on: true });
      await refreshVpns();
      term.write(`\x1b[2m── VPN up ──\x1b[0m\r\n`);
    } catch (err) {
      term.write(`\x1b[31m── VPN failed: ${String(err)} ──\x1b[0m\r\n`);
    }
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
  const vpath = prefs.vpn_auto_disconnect === true ? vpnFor(s.name) : null;
  invoke(s.kind === "rdp" ? "close_rdp_session" : "close_session", { id }).catch(() => {});
  s.unlisten.forEach((f) => f());
  s.term?.dispose();
  s.host.remove();
  sessions.delete(id);
  if (activeId === id) activeId = [...sessions.keys()].pop() ?? null;
  showTab();
  renderTabs();
  renderTree();

  // Only once nothing else in that folder is still connected.
  if (vpath && ![...sessions.values()].some((o) => vpnFor(o.name) === vpath)) {
    invoke("vpn_toggle", { path: vpath, on: false }).then(refreshVpns).catch(() => {});
  }
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
    requestAnimationFrame(() => { s.fit?.fit(); (s.term ?? s.canvas).focus(); });
  }
  renderDetail();
}

function renderTabs() {
  // The browse tab is the crumb — it names the selected folder and counts it.
  const label = group === null ? "All jacks" : group.split("/").join(" / ");
  const browse = `<div class="tab" data-id="" aria-selected="${activeId === null}">
      ${icon("layers")}<span class="lbl">${esc(label)}</span><span class="n">${shown.length}</span></div>`;
  tabsEl.innerHTML = browse + [...sessions.values()].map((s) => `
      <div class="tab ${s.dead ? "dead" : ""}" data-id="${s.id}" aria-selected="${s.id === activeId}">
        <span class="dot ${s.dead ? "down" : "up"}"></span>
        ${s.kind === "rdp" ? `<span class="tabkind">${icon("monitor")}</span>` : ""}
        <span class="lbl">${esc(s.name)}</span>
        <span class="x" data-close="${s.id}" data-tip="Close  ⌘W">${icon("x")}</span>
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
  if (activeId !== null) sessions.get(activeId)?.fit?.fit();
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
const rdpPasswords = new Map();

async function openRdpSession(name) {
  const j = all.find((x) => x.name === name);
  if (!j?.user) return alertish(`"${name}" needs a user to sign in with`);

  let password = rdpPasswords.get(name);
  if (password === undefined) {
    password = await ask(`Password for ${j.user}@${j.host}`, "", "Connect", "password");
    if (!password) return;
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
      id, name, password, width: 1280, height: 1024, onTile: chan,
    });
    // The server picks the size; asking for one is only a suggestion.
    canvas.width = screen.width;
    canvas.height = screen.height;
    const held = pending;
    pending = null;
    held.forEach(paint);
    rdpPasswords.set(name, password);
  } catch (err) {
    s.dead = true;
    // A rejected password must not be remembered, or the next attempt reuses it.
    rdpPasswords.delete(name);
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
