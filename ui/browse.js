// Classic script, no bundler - see the load order in ui/index.html.
// The device browser: folder tree, list, detail pane, command palette.

// ── sidebar tree ───────────────────────────────────────────────────────────
// A tag of "prod/eu/web" nests three deep; a jack counts toward every ancestor,
// and toward more than one branch if it carries more than one tag.
function buildTree(jacks, space) {
  const root = new Map();
  const placed = jacks.flatMap((j) => j.folders.map((f) => [j, f]));
  for (const p of pending.values()) if ((p.space ?? null) === space) placed.push([null, p.path]);
  for (const [j, folder] of placed) {
    {
      let level = root, path = "";
      for (const part of folder.split("/").map((p) => p.trim()).filter(Boolean)) {
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

// Your own list always exists, even empty - it's the config file. The rest come
// from the files beside it rather than from the devices, so a space you just made
// and haven't filled yet is still there.
function spacesOf() {
  const out = [null];
  const add = (s) => { if (!out.includes(s ?? null)) out.push(s ?? null); };
  for (const j of all) add(j.space);
  for (const s of spaces) add(s);
  for (const p of pending.values()) add(p.space);
  return out;
}

// Nested folders sort ahead of flat ones so a hierarchy doesn't get buried among
// alphabetically-interleaved single names. They are the same kind of thing either
// way - a folder is just a string a device carries.
function renderTree() {
  const live = new Set([...sessions.values()].filter((s) => !s.dead).map((s) => s.name));
  const names = (js) => new Set(js.map((j) => j.name));
  const rows = [
    row({ name: "All jacks", members: names(all), children: new Map() }, 0, "layers", live, null),
  ];

  const walk = (level, depth, space) => {
    for (const node of [...level.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      rows.push(row(node, depth, undefined, live, { space, path: node.path }));
      if (expanded.has(gkey({ space, path: node.path }))) walk(node.children, depth + 1, space);
    }
  };

  // One space is everyone's normal case, and a header above your only list is a row
  // that says nothing. The folders sit at the top level until there's a second space.
  const spaces = spacesOf();
  const nested = spaces.length > 1;
  if (nested) rows.push(`<div class="tree-sep"></div>`);
  for (const space of spaces) {
    const mine = all.filter((j) => (j.space ?? null) === space);
    // The space's own folders are its children, so its row folds like any other and
    // a space with nothing but loose devices correctly has nothing to fold.
    const tree = buildTree(mine, space);
    if (nested) {
      rows.push(row({ name: space ?? "Private", members: names(mine), children: tree },
                    0, "box", live, { space, path: null }));
      if (!expanded.has(gkey({ space, path: null }))) continue;
    }
    const roots = [...tree.values()].sort(
      (a, b) => (b.children.size > 0) - (a.children.size > 0) || a.name.localeCompare(b.name),
    );
    for (const node of roots) {
      rows.push(row(node, nested ? 1 : 0, undefined, live, { space, path: node.path }));
      if (expanded.has(gkey({ space, path: node.path }))) walk(node.children, nested ? 2 : 1, space);
    }
  }

  treeEl.innerHTML = rows.join("");
}

function row(node, depth, glyph, live, id) {
  const kids = node.children.size > 0;
  const open = expanded.has(gkey(id));
  const g = glyph ?? (kids && open ? "folder-open" : "folder");
  // The dot is always in the layout so it can carry the auto margin; it is only
  // painted when something under this node has a session open.
  const on = live && [...node.members].some((n) => live.has(n));
  return `<div class="group" data-space="${esc(id?.space ?? "")}" data-path="${esc(id?.path ?? "")}"
       data-group="${id ? "1" : ""}" data-has-kids="${kids}"
       aria-current="${sameGroup(group, id)}" style="padding-left:${8 + depth * 13}px">
    <span class="twist ${kids ? "" : "leaf"} ${open ? "open" : ""}">${icon("chevron-right")}</span>
    <span class="gi">${icon(g)}</span>
    <span class="label">${esc(node.name)}</span>
    <span class="live ${on ? "on" : ""}"${on ? ' data-tip="A session is open in here"' : ""}></span>
    <span class="n">${node.members.size}</span>
  </div>`;
}

// Something nested under it makes it a folder; a flat one is just a label. Same
// test the sidebar splits Folders from Tags on, so the wording matches the tree.
/// What the status dot says. "checking" and "unknown" are two different answers: one
/// is waiting on a sweep that is running, the other is a device nothing is ever going
/// to ask about because reachability checks are off. Here rather than in the three
/// places that draw a dot, because a fourth copy is how they start disagreeing.
function dotState(name) {
  if (prefs.probe === false) return "unknown";
  const p = probes.get(name);
  return !p ? "checking" : p.ms == null ? "down" : "up";
}

const inGroup = (j) =>
  group === null ? true
  : (j.space ?? null) !== group.space ? false
  : group.path === null ? true
  : j.folders.some((f) => f === group.path || f.startsWith(group.path + "/"));

// ── list ───────────────────────────────────────────────────────────────────
function render() {
  renderTree();
  shown = all.filter(inGroup);
  // Both the device sheet and the settings sheet suggest jump targets, so it's
  // filled here rather than by whichever one happens to open first.
  $("jacknames").innerHTML = all.map((x) => `<option value="${esc(x.name)}">`).join("");
  $("sshkeys").innerHTML = sshKeys.map((k) => `<option value="${esc(k)}">`).join("");

  renderTabs();
  searchBtn.innerHTML = `${icon("search")}Search<kbd>${chord("k")}</kbd>`;
  $("viewmode").innerHTML = listMode === "map" ? `${icon("list")}List` : `${icon("share-2")}Map`;
  $("viewmode").dataset.tip = listMode === "map"
    ? "Back to the flat list" : "Group by the route to each device";
  $("newjack").innerHTML = `${icon("plus")}Device<kbd>${chord("n")}</kbd>`;
  $("newgroup").innerHTML = icon("folder-plus");
  $("newgroup").dataset.tip = "New folder";
  $("newspace").innerHTML = icon("box");
  $("newspace").dataset.tip = "New space";
  $("editcfg").innerHTML = icon("file-pen-line");
  $("editcfg").dataset.tip = `Open the config file  ${chord("e")}`;
  $("settings").innerHTML = icon("settings");
  $("settings").dataset.tip = `Settings  ${chord(",")}`;
  // A stuck sync means your edits are not reaching anyone and it needs an answer from
  // you - so it shows on the button that leads there, not only inside the sheet.
  const stuck = teams.map((t) => TEAM_STUCK[t.state]).find(Boolean);
  $("settings").classList.toggle("warn", !!stuck);
  if (stuck) $("settings").dataset.tip = stuck;

  if (!shown.length) {
    listEl.innerHTML = all.length
      ? `<p class="empty">nothing here</p>`
      : `<div class="firstrun">
          <span class="fr-mark">${icon("server")}</span>
          <h3>No devices yet</h3>
          <p>Add one here, or write the file by hand - patchbay creates it either way,
             and keeps your comments and formatting if you edit it later.</p>
          <div class="mono">${esc(cfgPath)}</div>
          <div class="btns">
            <button class="primary" data-first="new">${icon("plus")}Add a device</button>
            <button class="ghost" data-first="import">${icon("download")}Import from ssh config</button>
            <button class="ghost" data-first="cfg">${icon("file-pen-line")}Open config file</button>
          </div>
        </div>`;
    renderDetail();   // a session tab still has something to describe
    return;
  }
  sel = Math.min(sel, shown.length - 1);
  listEl.innerHTML = listMode === "map" ? mapHtml() : shown.map((j, i) => jackRow(j, i)).join("");
  paintRows();
  renderDetail();
  listEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

/// One row, drawn the same whichever way the column is listing - so selection, the
/// double-click and the whole context menu keep working in the map without knowing it
/// exists. `i` indexes `shown`, which is what every handler reads.
function jackRow(j, i, depth = 0) {
  const state = dotState(j.name);
  // `readable()` nudges a brand hex against the *panel*, but the selected row is a
  // solid block of accent - Synology's navy clears 3:1 there and vanishes here. So
  // the row's own white wins on that one row, the way .host and .folder already do.
  const tint = i === sel ? null : osColor(j.os);
  return `<div class="jack" data-i="${i}" aria-selected="${i === sel}"${
    depth ? ` style="margin-left:${depth * 18}px"` : ""}>
    ${depth ? `<span class="hoparm">${icon("corner-down-right")}</span>` : ""}
    <span class="dot ${state}"></span>
    <span class="os"${j.os ? ` data-tip="${esc(j.os)}"` : ""}${
      tint ? ` style="color:${esc(tint)}"` : ""}>${osIcon(j.os)}</span>
    <span class="name">${esc(j.name)}</span>
    <span class="host">${esc(j.user ? j.user + "@" + j.host : j.host)}${j.port ? ":" + j.port : ""}</span>
    ${j.url ? `<span class="web" data-tip="${esc(j.url)}" data-tip-at="right">${icon("globe")}</span>` : ""}
    <span class="folders">${j.folders.map((f) => `<span class="folder">${esc(f.split("/").pop())}</span>`).join("")}</span>
  </div>`;
}

// ── map ────────────────────────────────────────────────────────────────────
// The same rows, grouped by the route to them instead of by the folder they were
// filed in. A bastion and the six machines behind it are one branch here even when
// those six live in six different folders - which is the thing a name tree can't show
// and nothing else in this category draws, because nothing else resolves the chain.

/// A jack's chain is a path, never a graph, so this is a tree and not a diagram.
function chainTree(js) {
  const root = { kids: new Map(), leaves: [] };
  for (const j of js) {
    let at = root;
    for (const h of j.hops) {
      if (!at.kids.has(h)) at.kids.set(h, { name: h, kids: new Map(), leaves: [] });
      at = at.kids.get(h);
    }
    at.leaves.push(j);
  }
  return root;
}

function mapHtml() {
  const at = (j) => shown.indexOf(j);
  const tree = chainTree(shown);

  const walk = (node, depth, blocked) => {
    // A hop that is itself a device is drawn as its own row heading the branch, not
    // repeated below it as one of the things reached directly.
    const rows = node.leaves
      .filter((j) => !node.kids.has(j.name))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((j) => jackRow(j, at(j), depth));

    for (const kid of [...node.kids.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      const via = shown.find((j) => j.name === kid.name);
      const p = probes.get(kid.name);
      // The one thing the chain buys us: a bastion that isn't answering explains
      // everything behind it, so the branch says so once instead of every row
      // underneath it showing its own unrelated-looking dot.
      const down = blocked || (p && p.ms == null);
      const behind = kid.leaves.length + kid.kids.size;
      rows.push(`<div class="hop ${down ? "blocked" : ""}" style="margin-left:${depth * 18}px">
        ${via ? jackRow(via, at(via)) : `<div class="jack hopraw">
          <span class="dot unknown"></span><span class="os">${icon("waypoints")}</span>
          <span class="name">${esc(kid.name)}</span>
          <span class="host">not in your list</span></div>`}
        <div class="hopnote">${icon("share-2")}${behind} behind ${esc(kid.name)}${
          down ? ` · not reachable while ${esc(kid.name)} is down` : ""}</div>
        ${walk(kid, depth + 1, down)}
      </div>`);
    }
    return rows.join("");
  };

  const html = walk(tree, 0, false);
  return html || `<p class="empty">nothing here</p>`;
}

function renderDetail() {
  // A live session tab wins: the pane describes what you're typing into.
  const live = activeId !== null ? sessions.get(activeId) : null;
  if (live) return renderJack(all.find((x) => x.name === live.name), live);
  if (detailMode === "group") return renderGroup();
  renderJack(shown[sel], null);
}

const groupLabel = () =>
  group === null ? "All jacks"
  : group.path === null ? (group.space ?? "Private")
  : group.path;

function renderGroup() {
  const members = all.filter(inGroup);
  const up = members.filter((j) => dotState(j.name) === "up").length;
  const down = members.filter((j) => dotState(j.name) === "down").length;
  const rest = members.length - up - down;
  // Whatever the dots are actually wearing, so the tally and the rows agree.
  const waiting = prefs.probe !== false;
  const open = [...sessions.values()].filter((s) => !s.dead && members.some((j) => j.name === s.name));
  // A space row has no folder to rename or delete.
  const real = group !== null && group.path !== null;
  // ...and a folder is not a space: what makes one a space is which file it is, and
  // when it belongs to a team, what the sync is doing. Neither was anywhere near the
  // row you clicked - the file was nowhere at all, the sync two clicks into settings.
  const space = group !== null && group.path === null ? group.space : undefined;
  const team = space === undefined ? null : teams.find((t) => t.space === space);
  const file = space === undefined ? null
    : spaceFiles.find((f) => (f.space ?? null) === space)?.path;

  detailEl.innerHTML = `
    <div class="d-name"><span class="d-os">${icon(
      real ? "folder-open" : group === null ? "layers" : "box")}</span>${esc(groupLabel())}</div>
    <div class="d-desc">${members.length} device${members.length === 1 ? "" : "s"}${
      real && group.path.includes("/") ? ` · in ${esc(group.path.slice(0, group.path.lastIndexOf("/")))}` : ""}</div>

    <div class="d-sec">${icon("plug")}Reachable</div>
    <div class="tallies">
      <span><i class="dot up"></i>${up} up</span>
      <span><i class="dot down"></i>${down} down</span>
      ${rest ? `<span><i class="dot ${waiting ? "checking" : "unknown"}"></i>${
        rest} ${waiting ? "still checking" : "not checked"}</span>` : ""}
    </div>

    ${open.length ? `<div class="d-sec">${icon("square-terminal")}Sessions</div>
      <div class="route">${open.map((s) => `<span class="last"><i class="pip"></i>${esc(s.name)}</span>`).join("")}</div>` : ""}

    ${space === undefined ? "" : `<div class="d-sec">${icon("box")}Space</div>
      <div class="d-row"><dt>kind</dt><dd>${
        !team ? "On this machine only"
        : team.code ? "A team's, mirrored through a server"
        : "Following a published list, read-only"}</dd></div>
      ${file ? `<div class="d-row"><dt>file</dt><dd>${esc(file)}</dd></div>` : ""}
      ${team ? `<div class="d-row"><dt>server</dt><dd>${esc(team.url)}</dd></div>
        <div class="d-row"><dt>sync</dt><dd>${esc(teamNote(team))}</dd></div>` : ""}`}
`;

  dActions.innerHTML = `
    <button class="primary" data-gact="new">${icon("plus")}Device</button>
    ${real ? `<button class="ghost" data-gact="rename" data-tip="Rename folder">${icon("pencil")}</button>
    <button class="ghost danger" data-gact="del" data-tip="Delete folder" data-tip-at="right">${icon("trash-2")}</button>` : ""}
    ${team?.state === "conflict" ? `
      <button class="ghost" data-gact="theirs">Take the team's</button>
      <button class="ghost" data-gact="mine">Push mine</button>` : ""}
    ${team ? `<button class="ghost" data-gact="sync" data-tip="Sync now" data-tip-at="right">${icon("rotate-cw")}</button>` : ""}`;
}

function renderJack(j, live) {
  if (!j) { detailEl.innerHTML = ""; dActions.innerHTML = ""; return; }
  const p = probes.get(j.name);
  // Same decider as the dot beside the row, so the pane and the list can't disagree
  // about whether this device is still being asked about.
  const said = dotState(j.name);
  const reach = said === "unknown" ? `<span style="color:var(--fg-faint)">not checked</span>`
    : said === "checking" ? `<span style="color:var(--fg-faint)">checking…</span>`
    : said === "down" ? `<span style="color:var(--down)">no answer</span> · ${esc(p.target)}`
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
      <div class="route">${mine.map((t) => `<span class="last"><i class="pip"></i>${
        /* A -R binds its port on the far end, so there is no local address to print. */
        t.local ? `127.0.0.1:${t.local}` : "held open on the far end"}
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
      <dl>${j.forward.map((f) => {
        // A bare forward is `-L`, the way `patchbay::forward_arg` reads it back.
        const m = /^(-[LRD])\s+(.+)$/.exec(f.trim());
        return `<div class="d-row"><dt>${m ? m[1] : "-L"}</dt><dd>${esc(m ? m[2] : f)}</dd></div>`;
      }).join("")}</dl>
      ${mine.length ? "" : `<div class="btns"><button class="ghost" data-act="forward"
        data-tip="Hold these open without a session">${icon("arrow-right-left")}Open forwards</button></div>`}` : ""}

    <div class="d-sec">${icon("plug")}Reachable</div>
    <div style="font-size:12.5px">${reach}</div>
    <div class="btns">
      <button class="ghost" data-act="ping" data-tip="${j.hops.length ? `Ping from ${esc(j.hops.at(-1))}` : "Ping this host"}">${icon("plug")}Ping</button>
      <button class="ghost" data-act="trace" data-tip="${j.hops.length ? `Trace from ${esc(j.hops.at(-1))}` : "Trace the route there"}">${icon("waypoints")}Trace</button>
    </div>

    ${j.ssh ? `<div class="d-sec">${icon("square-terminal")}Command</div>
    <div class="d-cmd">
      <div class="mono ${j.command.startsWith("ssh ") ? "" : "err"}">${esc(j.command)}</div>
      <button class="flat d-copy" data-act="copy" data-tip="Copy" data-tip-at="right">${icon("copy")}</button>
    </div>` : ""}
`;

  dActions.innerHTML = `
    ${live
      /* A web tab has nothing to disconnect from - it's a page, and you close it. */
      ? `<button class="primary" data-act="disconnect">${icon("x")}${
          live.dead || live.kind === "web" ? "Close" : "Disconnect"}</button>`
      : j.primary === "rdp" ? `<button class="primary" data-act="rdp">${icon("monitor")}Connect</button>`
      : j.primary === "vnc" ? `<button class="primary" data-act="vnc">${icon("screen-share")}Share screen</button>`
      : j.primary === "web" ? `<button class="primary" data-act="web">${icon("globe")}Open</button>`
      : j.primary === "sftp" ? `<button class="primary" data-act="files">${icon("folder")}Browse files</button>`
      : `<button class="primary" data-act="connect">${icon("square-terminal")}Connect</button>`}
    ${j.ssh && j.primary !== "ssh" ? `<button class="ghost" data-act="connect" data-tip="Connect over ssh">${icon("square-terminal")}</button>` : ""}
    ${j.ssh && j.primary !== "sftp" ? `<button class="ghost" data-act="files" data-tip="Browse files over sftp">${icon("folder")}</button>` : ""}
    ${j.url && j.primary !== "web" ? `<button class="ghost" data-act="web" data-tip="Open web UI">${icon("globe")}</button>` : ""}
    ${j.rdp && j.primary !== "rdp" ? `<button class="ghost" data-act="rdp" data-tip="Remote desktop">${icon("monitor")}</button>` : ""}
    ${j.vnc && j.primary !== "vnc" ? `<button class="ghost" data-act="vnc" data-tip="VNC">${icon("screen-share")}</button>` : ""}
    <button class="ghost" data-act="edit" data-tip="Edit device">${icon("pencil")}</button>`;
}

// Selection must not rebuild the list: replacing innerHTML destroys the row under
// the cursor, so the browser never pairs two clicks into a dblclick on one node.
function select(i) {
  detailMode = "jack";
  if (!shown.length) return;
  sel = (i + shown.length) % shown.length;
  paintRows();
  renderDetail();
  listEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

/// Selection and marks, painted without rebuilding the list - replacing innerHTML
/// destroys the row under the cursor, so the browser never pairs two clicks into a
/// dblclick on one node. Keyed on each row's own `data-i` rather than its position:
/// the map draws the same rows in the shape of the network, and a jump host that isn't
/// one of your devices is a row with no device behind it at all.
function paintRows() {
  for (const el of listEl.querySelectorAll(".jack[data-i]")) {
    const j = shown[+el.dataset.i];
    el.setAttribute("aria-selected", +el.dataset.i === sel);
    el.classList.toggle("marked", marked.has(j?.name));
  }
}

/// ⌘-click picks a row out, shift-click takes the run between it and the selected one -
/// the two gestures every list has. `sel` stays put as the anchor, so a second
/// shift-click extends from where you started rather than from the last one.
function markToggle(i) {
  const n = shown[i]?.name;
  if (!n) return;
  marked.has(n) ? marked.delete(n) : marked.add(n);
  select(i);
}
function markRange(i) {
  for (let k = Math.min(sel, i); k <= Math.max(sel, i); k++) marked.add(shown[k].name);
  paintRows();
}

/// What a bulk action applies to: the marks that are still in front of you. A device
/// marked in one folder and then filtered out of view is not part of what you asked for.
const markedHere = () => shown.filter((j) => marked.has(j.name));

const move = (d) => select(sel + d);

/// In a tab, like a terminal and like RDP. "Open in browser" is still on the context
/// menu, and `web_check` offers it when the page is one a webview can't show - an
/// appliance's self-signed certificate has no click-through here, only in a browser.
async function openWeb(name) {
  await openWebSession(name);
}

/// In a tab, like a terminal. "Open in Windows App" on the context menu is still
/// the handoff, for when someone wants their own client's settings.
async function openRdp(name) {
  await openRdpSession(name);
  await refreshTunnels();
}

// No tab of our own: the OS opens whatever registered `vnc://`, the way the system
// RDP client is handed a `.rdp`.
async function openVnc(name) {
  try { await invoke("open_vnc", { name }); } catch (e) { alertish(e); }
}

async function handOffRdp(name) {
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
  if (j.primary === "vnc") return openVnc(name);
  if (j.primary === "web") return openWeb(name);
  if (j.primary === "sftp") return openFilesSession(name);
  return connect(name);
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
// `seed` is set when you just start typing in the list - the keystroke isn't lost.
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
    ? rows.map((j, i) => {
        // Same rule as the list: the selected row is a solid block of accent, and a
        // brand colour tuned against the panel disappears on it.
        const tint = i === palSel ? null : osColor(j.os);
        return `<div class="jack" data-pi="${i}" aria-selected="${i === palSel}">
        <span class="os"${tint ? ` style="color:${esc(tint)}"` : ""}>${osIcon(j.os)}</span>
        <span class="name">${esc(j.name)}</span>
        <span class="host">${esc(j.host)}</span></div>`;
      }).join("")
    : `<p class="empty">no match</p>`;
  presultsEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}
