// Classic script, no bundler — see the load order in ui/index.html.
// The device browser: folder tree, list, detail pane, command palette.

// ── sidebar tree ───────────────────────────────────────────────────────────
// A tag of "prod/eu/web" nests three deep; a jack counts toward every ancestor,
// and toward more than one branch if it carries more than one tag.
function buildTree() {
  const root = new Map();
  const tagged = all.flatMap((j) => j.tags.map((t) => [j, t]));
  for (const p of pending) tagged.push([null, p]);
  for (const [j, tag] of tagged) {
    {
      let level = root, path = "";
      for (const part of tag.split("/").map((p) => p.trim()).filter(Boolean)) {
        path = path ? `${path}/${part}` : part;
        if (!level.has(part)) level.set(part, { name: part, path, members: new Set(), children: new Map() });
        const node = level.get(part);
        if (j) node.members.add(j.name);
        level = node.children;
      }
    }
  }
  return root;
}

// A root node with children is a folder; one without is a flat label. Splitting
// them stops the hierarchy being buried among alphabetically-interleaved tags.
function renderTree() {
  const tree = buildTree();
  const live = new Set([...sessions.values()].filter((s) => !s.dead).map((s) => s.name));
  const untagged = all.filter((j) => !j.tags.length).map((j) => j.name);
  const leaf = (name, path, members, glyph) =>
    row({ name, path, members: new Set(members), children: new Map() }, 0, glyph, live);

  const rows = [leaf("All jacks", null, all.map((j) => j.name), "layers")];

  const walk = (level, depth) => {
    for (const node of [...level.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      rows.push(row(node, depth, undefined, live));
      if (expanded.has(node.path)) walk(node.children, depth + 1);
    }
  };

  const roots = [...tree.values()].sort((a, b) => a.name.localeCompare(b.name));
  const folders = roots.filter((n) => n.children.size);
  const tags = roots.filter((n) => !n.children.size);

  if (folders.length) {
    rows.push(`<div class="side-title">Folders</div>`);
    for (const node of folders) {
      rows.push(row(node, 0, undefined, live));
      if (expanded.has(node.path)) walk(node.children, 1);
    }
  }

  if (tags.length || untagged.length) rows.push(`<div class="side-title">Tags</div>`);
  for (const node of tags) rows.push(row(node, 0, undefined, live));
  if (untagged.length) rows.push(leaf("Untagged", "\0untagged", untagged, "circle-off"));

  treeEl.innerHTML = rows.join("");
}

function row(node, depth, glyph, live) {
  const kids = node.children.size > 0;
  const open = expanded.has(node.path);
  const g = glyph ?? (kids ? (open ? "folder-open" : "folder") : "tag");
  // The dot is always in the layout so it can carry the auto margin; it is only
  // painted when something under this node has a session open.
  const on = live && [...node.members].some((n) => live.has(n));
  return `<div class="group" data-path="${esc(node.path ?? "")}" data-has-kids="${kids}"
       aria-current="${group === node.path}" style="padding-left:${8 + depth * 13}px">
    <span class="twist ${kids ? "" : "leaf"} ${open ? "open" : ""}">${icon("chevron-right")}</span>
    <span class="gi">${icon(g)}</span>
    <span class="label">${esc(node.name)}</span>
    <span class="live ${on ? "on" : ""}"${on ? ' data-tip="A session is open in here"' : ""}></span>
    ${vpnSwitch(node.path)}
    <span class="n">${node.members.size}</span>
  </div>`;
}

// Only folders named by a [vpn."..."] section get one.
function vpnSwitch(path) {
  const v = path && vpns.get(path);
  if (!v) return "";
  const busy = vpnBusy.has(path);
  const title = busy ? "working…"
    : v.up ? `VPN up${v.known ? "" : " (remembered, no check command)"} — click to disconnect`
    : "VPN down — click to connect";
  return `<span class="vpn ${v.up ? "on" : ""} ${busy ? "busy" : ""}"
     data-vpn="${esc(path)}" role="switch" aria-checked="${!!v.up}"
     data-tip="${esc(title)}" data-tip-at="right"><i></i></span>`;
}

const inGroup = (j) =>
  group === null ? true
  : group === "\0untagged" ? j.tags.length === 0
  : j.tags.some((t) => t === group || t.startsWith(group + "/"));

// ── list ───────────────────────────────────────────────────────────────────
function render() {
  renderTree();
  shown = all.filter(inGroup);

  renderTabs();
  searchBtn.innerHTML = `${icon("search")}Search<kbd>${chord("k")}</kbd>`;
  $("newjack").innerHTML = `${icon("plus")}Device<kbd>${chord("n")}</kbd>`;
  $("newgroup").innerHTML = icon("folder-plus");
  $("newgroup").dataset.tip = "New folder";
  $("newgroup").dataset.tipAt = "left";
  $("editcfg").innerHTML = icon("file-pen-line");
  $("editcfg").dataset.tip = `Open the config file  ${chord("e")}`;
  $("editcfg").dataset.tipAt = "left";
  $("settings").innerHTML = icon("cog");
  $("settings").dataset.tip = `Settings  ${chord(",")}`;
  $("settings").dataset.tipAt = "right";

  if (!shown.length) {
    listEl.innerHTML = all.length
      ? `<p class="empty">nothing here</p>`
      : `<div class="firstrun">
          <span class="fr-mark">${icon("server")}</span>
          <h3>No devices yet</h3>
          <p>Add one here, or write the file by hand — patchbay creates it either way,
             and keeps your comments and formatting if you edit it later.</p>
          <div class="mono">${esc(cfgPath)}</div>
          <div class="btns">
            <button class="primary" data-first="new">${icon("plus")}Add a device</button>
            <button class="ghost" data-first="cfg">${icon("file-pen-line")}Open config file</button>
          </div>
        </div>`;
    renderDetail();   // a session tab still has something to describe
    return;
  }
  sel = Math.min(sel, shown.length - 1);
  listEl.innerHTML = shown.map((j, i) => {
    const p = probes.get(j.name);
    const state = !p ? "unknown" : p.ms == null ? "down" : "up";
    return `<div class="jack" data-i="${i}" aria-selected="${i === sel}">
      <span class="dot ${state}"></span>
      <span class="os"${j.os ? ` data-tip="${esc(j.os)}"` : ""}${
        osColor(j.os) ? ` style="color:${esc(osColor(j.os))}"` : ""}>${osIcon(j.os)}</span>
      <span class="name">${esc(j.name)}</span>
      <span class="host">${esc(j.user ? j.user + "@" + j.host : j.host)}${j.port ? ":" + j.port : ""}</span>
      ${j.url ? `<span class="web" data-tip="${esc(j.url)}" data-tip-at="right">${icon("globe")}</span>` : ""}
      <span class="tags">${j.tags.map((t) => `<span class="tag">${esc(t.split("/").pop())}</span>`).join("")}</span>
    </div>`;
  }).join("");
  renderDetail();
  listEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

function renderDetail() {
  // A live session tab wins: the pane describes what you're typing into.
  const live = activeId !== null ? sessions.get(activeId) : null;
  if (live) return renderJack(all.find((x) => x.name === live.name), live);
  if (detailMode === "group") return renderGroup();
  renderJack(shown[sel], null);
}

const groupLabel = () =>
  group === null ? "All jacks" : group === "\0untagged" ? "Untagged" : group;

function renderGroup() {
  const members = all.filter(inGroup);
  const state = (j) => {
    const p = probes.get(j.name);
    return !p ? "unknown" : p.ms == null ? "down" : "up";
  };
  const up = members.filter((j) => state(j) === "up").length;
  const down = members.filter((j) => state(j) === "down").length;
  const unknown = members.length - up - down;
  const open = [...sessions.values()].filter((s) => !s.dead && members.some((j) => j.name === s.name));
  const real = group !== null && group !== "\0untagged";
  const v = real && vpns.get(group);
  const busy = real && vpnBusy.has(group);

  detailEl.innerHTML = `
    <div class="d-name"><span class="d-os">${icon(real ? "folder-open" : "layers")}</span>${esc(groupLabel())}</div>
    <div class="d-desc">${members.length} device${members.length === 1 ? "" : "s"}${
      real && group.includes("/") ? ` · in ${esc(group.slice(0, group.lastIndexOf("/")))}` : ""}</div>

    <div class="d-sec">${icon("plug")}Reachable</div>
    <div class="tallies">
      <span><i class="dot up"></i>${up} up</span>
      <span><i class="dot down"></i>${down} down</span>
      ${unknown ? `<span><i class="dot unknown"></i>${unknown} unknown</span>` : ""}
    </div>

    ${open.length ? `<div class="d-sec">${icon("square-terminal")}Sessions</div>
      <div class="route">${open.map((s) => `<span class="last"><i class="pip"></i>${esc(s.name)}</span>`).join("")}</div>` : ""}

    ${v ? `<div class="d-sec">${icon("plug")}VPN</div>
      <div style="font-size:12.5px">${
        v.up ? `<span style="color:var(--up)">connected</span>` : `<span style="color:var(--fg-dim)">disconnected</span>`
      }${v.known ? "" : ` <span style="color:var(--fg-faint)">· remembered, no check command</span>`}</div>
      <div class="btns">
        <button class="${v.up ? "" : "primary"}" data-gact="vpn" ${busy ? "disabled" : ""}>
          ${icon("plug")}${busy ? "working…" : v.up ? "Disconnect" : "Connect VPN"}</button>
      </div>` : ""}

`;

  dActions.innerHTML = `
    <button class="primary" data-gact="new">${icon("plus")}Device</button>
    ${real ? `<button class="ghost" data-gact="vpnedit" data-tip="${v ? "VPN settings" : "Add a VPN"}">${icon("plug")}</button>
    <button class="ghost" data-gact="rename" data-tip="Rename folder">${icon("pencil")}</button>
    <button class="ghost danger" data-gact="del" data-tip="Delete folder" data-tip-at="right">${icon("trash-2")}</button>` : ""}`;
}

function renderJack(j, live) {
  if (!j) { detailEl.innerHTML = ""; dActions.innerHTML = ""; return; }
  const p = probes.get(j.name);
  const reach = prefs.probe === false ? `<span style="color:var(--fg-faint)">not checked</span>`
    : !p ? `<span style="color:var(--fg-faint)">checking…</span>`
    : p.ms == null ? `<span style="color:var(--down)">no answer</span> · ${esc(p.target)}`
    : `<span style="color:var(--up)">up</span> · ${esc(p.target)} · ${p.ms}ms`;

  const stops = [...j.hops, j.user ? `${j.user}@${j.host}` : j.host];
  const mine = tunnels.filter((t) => t.jack === j.name);
  detailEl.innerHTML = `
    <div class="d-name"><span class="d-os"${
      osColor(j.os) ? ` style="color:${esc(osColor(j.os))}"` : ""}>${osIcon(j.os)}</span>${esc(j.name)}</div>
    ${j.desc ? `<div class="d-desc">${esc(j.desc)}</div>` : `<div class="d-desc"></div>`}

    <div class="d-sec">${icon("server")}Target</div>
    <dl>
      <div class="d-row"><dt>host</dt><dd>${esc(j.host)}</dd></div>
      ${j.user ? `<div class="d-row"><dt>user</dt><dd>${esc(j.user)}</dd></div>` : ""}
      ${j.port ? `<div class="d-row"><dt>port</dt><dd>${j.port}</dd></div>` : ""}
      ${j.key ? `<div class="d-row"><dt>key</dt><dd>${esc(j.key)}</dd></div>` : ""}
      ${j.url ? `<div class="d-row"><dt>web</dt><dd>${esc(j.url)}</dd></div>` : ""}
      ${j.rdp ? `<div class="d-row"><dt>rdp</dt><dd>${j.rdp}</dd></div>` : ""}
    </dl>
    ${mine.length ? `<div class="d-sec">${icon("waypoints")}Tunnel</div>
      <div class="route">${mine.map((t) => `<span class="last"><i class="pip"></i>127.0.0.1:${t.local}
        <i class="arm">via ${esc(t.via)}</i></span>`).join("")}</div>
      <div class="btns"><button class="ghost danger" data-act="untunnel"
        data-tip="Close the forward">${icon("unplug")}Close tunnel</button></div>` : ""}

    <div class="d-sec">${icon("waypoints")}Route</div>
    <div class="route">
      <span><i class="pip"></i>this machine</span>
      ${stops.map((h, i) => `<span class="${i === stops.length - 1 ? "last" : ""}">
        <i class="pip"></i>${esc(h)}${i < stops.length - 1 ? `<i class="arm">jump</i>` : ""}</span>`).join("")}
    </div>

    ${j.forward.length ? `<div class="d-sec">${icon("arrow-right-left")}Forwards</div>
      <dl>${j.forward.map((f) => `<div class="d-row"><dt>-L</dt><dd>${esc(f)}</dd></div>`).join("")}</dl>` : ""}

    <div class="d-sec">${icon("plug")}Reachable</div>
    <div style="font-size:12.5px">${reach}</div>

    ${j.ssh ? `<div class="d-sec">${icon("square-terminal")}Command</div>
    <div class="mono ${j.command.startsWith("ssh ") ? "" : "err"}">${esc(j.command)}</div>` : ""}
`;

  dActions.innerHTML = `
    ${live
      ? `<button class="primary" data-act="disconnect">${icon("x")}${live.dead ? "Close" : "Disconnect"}</button>`
      : j.primary === "rdp" ? `<button class="primary" data-act="rdp">${icon("monitor")}Connect</button>`
      : j.primary === "web" ? `<button class="primary" data-act="web">${icon("globe")}Open</button>`
      : `<button class="primary" data-act="connect">${icon("square-terminal")}Connect</button>`}
    ${j.ssh && j.primary !== "ssh" ? `<button class="ghost" data-act="connect" data-tip="Connect over ssh">${icon("square-terminal")}</button>` : ""}
    ${j.url && j.primary !== "web" ? `<button class="ghost" data-act="web" data-tip="Open web UI">${icon("globe")}</button>` : ""}
    ${j.rdp && j.primary !== "rdp" ? `<button class="ghost" data-act="rdp" data-tip="Remote desktop">${icon("monitor")}</button>` : ""}
    <button class="ghost" data-act="edit" data-tip="Edit device">${icon("pencil")}</button>
    ${j.ssh ? `<button class="ghost" data-act="copy" data-tip="Copy ssh command" data-tip-at="right">${icon("copy")}</button>` : ""}`;
}

// Selection must not rebuild the list: replacing innerHTML destroys the row under
// the cursor, so the browser never pairs two clicks into a dblclick on one node.
function select(i) {
  detailMode = "jack";
  if (!shown.length) return;
  sel = (i + shown.length) % shown.length;
  listEl.querySelectorAll(".jack").forEach((el, n) => el.setAttribute("aria-selected", n === sel));
  renderDetail();
  listEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

const move = (d) => select(sel + d);

async function openWeb(name) {
  try { await invoke("open_url", { name }); } catch (e) { alertish(e); }
}

async function openRdp(name) {
  try {
    await invoke("open_rdp", { name });
    await refreshTunnels();
  } catch (e) { alertish(e); }
}

async function refreshTunnels() {
  try { tunnels = await invoke("tunnels"); render(); } catch { /* none is normal */ }
}

/// What Enter, a double-click and the palette do: ssh if it has it, else remote
/// desktop, else the web UI. Connect is meaningless on a web-only NAS.
function primary(name) {
  const j = all.find((x) => x.name === name);
  if (!j) return;
  if (j.primary === "rdp") return openRdp(name);
  if (j.primary === "web") return openWeb(name);
  return connect(name);
}

/// The most specific folder VPN covering this device, or null.
function vpnFor(name) {
  const j = all.find((x) => x.name === name);
  if (!j) return null;
  let best = null;
  for (const path of vpns.keys()) {
    const covers = j.tags.some((t) => t === path || t.startsWith(path + "/"));
    if (covers && (!best || path.length > best.length)) best = path;
  }
  return best;
}

// In-app unless the preference says otherwise; `inTerminal` forces the handoff.
async function connect(name, inTerminal = prefs.connect_in_terminal === true) {
  if (!inTerminal) return openSession(name);
  try {
    await invoke("connect", { name });
  } catch (e) {
    alertish(e);
  }
}

// ── command palette ────────────────────────────────────────────────────────
const palOpen = () => !paletteEl.hidden;
// `seed` is set when you just start typing in the list — the keystroke isn't lost.
function openPalette(seed = "") {
  paletteEl.hidden = false;
  pq.value = seed; palSel = 0;
  $("pq-icon").innerHTML = icon("search");
  renderPalette();
  pq.focus();
  pq.setSelectionRange(seed.length, seed.length);
}
function closePalette() { paletteEl.hidden = true; }

function palMatches() {
  const f = pq.value.trim().toLowerCase();
  return all.filter((j) => hit(j, f)).slice(0, 40);
}
function renderPalette() {
  const rows = palMatches();
  palSel = Math.min(palSel, Math.max(0, rows.length - 1));
  presultsEl.innerHTML = rows.length
    ? rows.map((j, i) => `<div class="jack" data-pi="${i}" aria-selected="${i === palSel}">
        <span class="name">${esc(j.name)}</span>
        <span class="host">${esc(j.host)}</span></div>`).join("")
    : `<p class="empty">no match</p>`;
  presultsEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}
