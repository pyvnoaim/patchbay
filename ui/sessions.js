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
