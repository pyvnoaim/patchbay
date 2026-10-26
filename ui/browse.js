// The device browser: the tree of vaults, folders and devices, the detail of whatever
// is selected in it, and the command palette.
// Classic script sharing one global scope; see the load order in ui/index.html.
//
// The tree is the list. There is no middle column of devices: a folder unfolds to its folders and
// then its devices, and `shown` is the device rows as drawn, with `shownIn` the folder each sits
// in. A device filed twice is two rows, so the selection is a row: `sel` indexes `shown`, and
// `selKey` (name plus folder) is what finds it again after a folder unfolds above it. Whatever is
// selected is described by one renderer in two places: wide in `#wide` while browsing, and in the
// narrow `#detail` pane beside a session, which is hidden until the Details chip asks for it.
// `activeId === null` is that browse view; there is no tab for it, and a click in the tree is how
// you get back to it (`browse()`). The map is the selected folder drawn by route, in place of the
// detail.
//
// Marks are names, the selection is an index. ⌘-click and shift-click pick rows out into `marked`;
// the tree is rebuilt under them by every render, so an index would follow whatever moved into that
// row. The first ⌘-click also marks the selected row, the way shift-click always has: otherwise
// three rows are lit, the dock says two, and the blue one is the one Delete spares. `markedHere()`
// is what a bulk action applies to - a mark on a device you have since filtered out of view is not
// part of what was asked for. Broadcast is one of those actions: `openBroadcast()` in `sessions.js`
// opens every ssh mark as its own pane inside a shared tab, and their `s.bcast.on` fans one
// `onData` out to every live sibling. Non-ssh marks are named in a pill and skipped - a webview or
// an RDP canvas mixed into the grid layout is a much bigger change than the feature is.
//
// A pane being typed into is never redrawn. `renderDetail` returns early when the focus is inside
// it: the probe sweep re-renders every thirty seconds, and `innerHTML` takes a half-written folder
// note with it.
//
// A drag is pointer events, not HTML5 drag and drop. The window's file-drop handler (uploads into a
// files tab) takes the native drag over on some platforms, and a web tab is an OS view above the
// page that would swallow a `dragend`. `pointerdown` on a row, a six-pixel threshold,
// `elementFromPoint` for the folder under the pointer, and the click that ends a drag is stopped in
// the capture phase so it doesn't reselect. A drop writes only `folders`, through `set_folders`, so
// a hand-written key on the device survives; `moveJacks` is the one place the semantics live - a
// *move* out of the folder you are looking at, an *add* from All devices.

// ── sidebar tree ───────────────────────────────────────────────────────────
// "prod/eu/web" nests three deep; a jack counts toward every ancestor and every
// branch it is filed in. One tree per vault: a folder is in one of them.
const normPath = (f) =>
  f
    .split("/")
    .map((p) => p.trim())
    .filter(Boolean)
    .join("/");

function buildTree(jacks, vault) {
  const root = new Map();
  const placed = jacks.flatMap((j) => j.folders.map((f) => [j, f]));
  for (const p of pending.values())
    if ((p.vault ?? undefined) === vault) placed.push([null, p.path]);
  for (const [j, folder] of placed) {
    let level = root,
      path = "";
    for (const part of normPath(folder).split("/").filter(Boolean)) {
      path = path ? `${path}/${part}` : part;
      if (!level.has(part))
        level.set(part, { name: part, path, members: new Set(), children: new Map() });
      const node = level.get(part);
      if (j) node.members.add(j.name);
      level = node.children;
    }
  }
  return root;
}

// Every folder a vault has, parents included, for "Move to…" to offer.
function folderPaths(vault) {
  const out = new Set();
  const named = [
    ...all.filter((j) => inVault(j, vault)).flatMap((j) => j.folders),
    ...[...pending.values()].filter((p) => (p.vault ?? undefined) === vault).map((p) => p.path),
  ];
  for (const f of named) {
    const parts = normPath(f).split("/").filter(Boolean);
    for (let i = 1; i <= parts.length; i++) out.add(parts.slice(0, i).join("/"));
  }
  return out;
}
// What a folder box suggests: one vault's, because the other's are in another file and
// a name picked from there would make a new folder here.
function offerFolders(vault) {
  $("folderlist").innerHTML = [...folderPaths(vault)]
    .sort()
    .map((f) => `<option value="${esc(f)}">`)
    .join("");
}
// The same, across both vaults, as the ids a folder is selected by.
const folderIds = () =>
  vaultIds().flatMap((vault) => [...folderPaths(vault)].map((path) => ({ path, vault })));

// What a tree row stands for.
const idOf = (el) => ({ path: el.dataset.path || null, vault: el.dataset.vault || undefined });
const keyAt = (i) => (shown[i] ? `${shown[i].name}\n${gkey(shownIn[i])}` : null);

// The tree is the list: a folder unfolds to its folders and then its devices, and
// `shown` is those device rows in the order they are drawn. A device filed in two
// folders is two rows, which is why the selection is a row and not a name.
function renderTree() {
  const live = new Set([...sessions.values()].filter((s) => !s.dead).map((s) => s.name));
  const rows = [];
  shown = [];
  shownIn = [];
  const place = (j, id) => {
    shownIn.push(id);
    return shown.push(j) - 1;
  };

  for (const vault of vaultIds()) {
    const js = all.filter((j) => inVault(j, vault));
    if (vault) rows.push(vaultRow(vault, js.length));
    const walk = (level, depth) => {
      for (const node of [...level.values()].sort((a, b) => collate(a.name, b.name))) {
        const id = { path: node.path, vault };
        const here = js.filter((j) => j.folders.some((f) => normPath(f) === node.path));
        rows.push(row(node, depth, live, id));
        if (!expanded.has(gkey(id))) continue;
        walk(node.children, depth + 1);
        for (const j of here) rows.push(devRow(j, place(j, id), depth + 1, live, id));
      }
    };
    walk(buildTree(js, vault), 0);
    const root = { path: null, vault };
    for (const j of js.filter((j) => !j.folders.some((f) => normPath(f)))) {
      rows.push(devRow(j, place(j, root), 0, live, root));
    }
    // Said where the devices would be, not in place of the whole window: yours still open.
    // A list closed for want of a licence says so, with the way to add one; a share that
    // is away gets another look. A licence in its grace week warns while the list opens.
    const lic = vaults.on_sale ? vaults.licence : null;
    const closed = lic && (lic.state === "unlicensed" || lic.state === "ended");
    if (vault === "team" && vaults.away) {
      rows.push(
        `<div class="vault-away">${esc(vaults.away)}${
          closed
            ? `<button type="button" class="ghost" data-licence="1">${icon("key-round")}Licence</button>`
            : `<button type="button" class="ghost" data-retry="1">${icon("rotate-cw")}Look again</button>`
        }</div>`,
      );
    } else if (vault === "team" && lic?.state === "grace") {
      rows.push(`<div class="vault-away">${esc(`${lic.company}'s licence ended on ${licDay(lic.until)}. The team's list keeps opening until ${licDay(lic.closes)}.`)}
        <button type="button" class="ghost" data-licence="1">${icon("key-round")}Licence</button></div>`);
    }
  }

  // The row that was selected, wherever it is now. Folded away, the detail falls back
  // to the folder rather than describing some other device.
  const at = selKey === null ? -1 : shown.findIndex((_, i) => keyAt(i) === selKey);
  if (at < 0 && detailMode === "jack") detailMode = "group";
  sel = at < 0 ? Math.min(sel, Math.max(0, shown.length - 1)) : at;
  treeEl.innerHTML = rows.join("");
  // The rows carry no selection of their own, so a tree is never left unpainted,
  // whoever asked for it.
  paintRows();
}

const indent = (depth) => `style="--depth:${depth}"`;
const idAttrs = (id) =>
  `data-path="${esc(id.path ?? "")}"${id.vault ? ` data-vault="${esc(id.vault)}"` : ""}`;

// The drive the shared list is on, for a row too narrow for its path; the header and the
// tooltip have the path. A shared drive or a mounted volume by its own name, a sync
// client's folder by the client ("GoogleDrive-me@acme.com" is "Google Drive"), and
// anything else by the folder the list is in.
function driveName(path) {
  return driveOf(path).name;
}
// The drive, and the folders from it down to the list: what Settings shows under a path
// too long for its field.
function driveOf(path) {
  const parts = path.split(/[\\/]/).filter(Boolean);
  // The folder after `word`, as long as it is a folder and not the list file itself.
  const at = (word) => {
    const i = parts.indexOf(word);
    return i >= 0 && i < parts.length - 2 ? i + 1 : -1;
  };
  const root = [at("Shared drives"), at("CloudStorage"), at("Volumes")].find((i) => i >= 0);
  if (root === undefined) return { name: parts.at(-2) ?? path, rest: parts.slice(-1) };
  const name =
    parts[root - 1] === "CloudStorage"
      ? parts[root].replace(/-.*/, "").replace("GoogleDrive", "Google Drive")
      : parts[root];
  return { name, rest: parts.slice(root + 1) };
}

// A vault's own row: who can see what is under it, and for the team's, where it is.
function vaultRow(vault, n) {
  const [glyph, name] = VAULTS[vault];
  const sub =
    vault === "mine"
      ? "Only on this machine"
      : vaults.away
        ? `<span class="dot down"></span>Not there`
        : `<span class="dot up"></span><span data-tip="${esc(vaults.team ?? "")}">${esc(driveName(vaults.team ?? ""))}</span>`;
  return `<div class="group vault ${vault}${vault === "team" && vaults.away ? " away" : ""}"
       data-vault="${vault}" data-path="" data-group="1" data-has-kids="false"
       role="treeitem" aria-level="1" aria-label="${esc(`${name}, ${n} device${n === 1 ? "" : "s"}`)}">
    <span class="badge">${icon(glyph)}</span>
    <span class="label"><b>${name}</b><small>${sub}</small></span>
    <span class="n">${n}</span>
  </div>`;
}

function row(node, depth, live, id) {
  const kids = node.children.size > 0 || node.members.size > 0;
  const open = expanded.has(gkey(id));
  // The dot is always in the layout for its auto margin; painted only with a live session.
  const on = [...node.members].some((n) => live.has(n));
  const [dot, say] = node.members.size
    ? reachSaid([...node.members].map((name) => ({ name })))
    : [];
  // A screen reader hears the folder, whether it is open, and how much of it answers.
  const heard = `${node.name}, ${node.members.size} device${node.members.size === 1 ? "" : "s"}${say ? `, ${say.toLowerCase()}` : ""}`;
  return `<div class="group" ${idAttrs(id)} data-group="1" data-has-kids="${kids}" ${indent(depth)}
    role="treeitem" aria-level="${depth + 1}" aria-label="${esc(heard)}"${kids ? ` aria-expanded="${open}"` : ""}>
    <span class="twist ${kids ? "" : "leaf"} ${open ? "open" : ""}">${icon("chevron-right")}</span>
    <span class="gi">${icon(kids && open ? "folder-open" : "folder")}</span>
    <span class="label">${esc(node.name)}</span>${
      say ? `<span class="dot ${dot}" data-tip="${esc(say)}"></span>` : ""
    }
    <span class="live ${on ? "on" : ""}"${on ? ' data-tip="A session is open in here"' : ""}></span>
    <span class="n">${node.members.size}</span>
  </div>`;
}

// What both kinds of device row end with: a session's dot, then how it is reached.
const hereTip = (name) =>
  `${present.get(name).join(", ")} ${present.get(name).length === 1 ? "is" : "are"} connected`;
const devTail = (j, live) => `
    <span class="live ${live.has(j.name) ? "on" : ""}"${live.has(j.name) ? ' data-tip="A session is open"' : ""}></span>${
      present.has(j.name)
        ? `<span class="here" data-tip="${esc(hereTip(j.name))}">${icon("users")}</span>`
        : ""
    }${
      // ssh is what a device is unless it says otherwise, so only the others are marked:
      // a glyph on every row said nothing and cost the name its last letters.
      KIND[j.primary] && j.primary !== "ssh"
        ? `<span class="kind" data-tip="${esc(KIND[j.primary][1])}">${icon(KIND[j.primary][0])}</span>`
        : ""
    }`;

// A device in the tree. Selection and marks are painted afterwards (`paintRows`), so
// a click never rebuilds the row under the pointer.
// The status a dot paints, in words: a dot alone says nothing to a screen reader, or to
// someone who can't tell its green from its grey.
const DOT_SAID = { up: "reachable", down: "out of reach", checking: "checking", unknown: "" };

function devRow(j, i, depth, live, id) {
  const tint = osColor(j.os);
  const heard = [
    j.label,
    DOT_SAID[dotState(j.name)],
    j.primary !== "ssh" && KIND[j.primary]?.[1],
    live.has(j.name) && "session open",
  ].filter(Boolean);
  return `<div class="jack dev" data-i="${i}" ${idAttrs(id)} ${indent(depth)}
    role="treeitem" aria-level="${depth + 1}" aria-label="${esc(heard.join(", "))}">
    <span class="dot ${dotState(j.name)}"></span>
    <span class="os"${j.os ? ` data-tip="${esc(j.os)}"` : ""}${
      tint ? ` style="color:${esc(tint)}"` : ""
    }>${osIcon(j.os)}</span>
    <span class="label">${esc(j.label)}</span>${devTail(j, live)}
  </div>`;
}

// What the status dot says. "checking" waits on a running sweep; "unknown" means
// reachability checks are off. One decider, so the dot, the pane and the tally agree.
function dotState(name) {
  if (prefs.probe === false) return "unknown";
  // A session that is up is a better answer than a sweep that has not come round yet.
  for (const s of sessions.values()) if (!s.dead && s.name === name) return "up";
  const p = probes.get(name);
  return !p ? "checking" : p.ms == null ? "down" : "up";
}

const inGroup = (j) =>
  !group ||
  ((group.vault === undefined || inVault(j, group.vault)) &&
    (group.path === null ||
      j.folders.some(
        (f) => normPath(f) === group.path || normPath(f).startsWith(group.path + "/"),
      )));

// ── everything, redrawn ────────────────────────────────────────────────────
function render() {
  // Not while a press is in flight: `innerHTML` takes away the row or button being
  // pressed, and the click that would have fired on it never does. `releasePress()`
  // runs the skipped render once the click has landed.
  if (pressing) {
    missedRender = true;
    return;
  }
  renderTree();
  // Both the device sheet and the settings sheet suggest jump targets, by the key a
  // file would say.
  $("jacknames").innerHTML = [...new Map(all.map((x) => [bareKey(x.name), x])).entries()]
    .map(
      ([key, x]) =>
        `<option value="${esc(key)}">${x.label === key ? "" : esc(`${x.label} · ${x.folders.join(", ")}`)}</option>`,
    )
    .join("");
  $("sshkeys").innerHTML = sshKeys.map((k) => `<option value="${esc(k)}">`).join("");

  renderTabs();
  searchBtn.innerHTML = `${icon("search")}Search<kbd>${chord("k")}</kbd>`;
  $("viewmode").innerHTML = listMode === "map" ? `${icon("x")}Close map` : `${icon("share-2")}Map`;
  $("viewmode").dataset.tip =
    listMode === "map" ? "Back to the detail" : "Draw this folder by the route to each device";
  $("newgroup").innerHTML = icon("folder-plus");
  $("newgroup").dataset.tip = "New folder";
  $("editcfg").innerHTML = icon("file-pen-line");
  $("editcfg").dataset.tip = `Open the config file  ${chord("e")}`;
  $("settings").innerHTML = icon("settings");
  $("settings").dataset.tip = `Settings  ${chord(",")}`;

  // The right-hand side is the detail, unless there is nothing to detail or the map
  // was asked for.
  const mapping = listMode === "map" || !all.length;
  listEl.hidden = !mapping;
  $("wide").hidden = mapping;
  if (!all.length) {
    listEl.innerHTML = `<div class="firstrun">
          <span class="fr-mark">${icon("server")}</span>
          <h3>No devices yet</h3>
          <p>Add one here, or write the file by hand - patchbay creates it either way,
             and keeps your comments and formatting if you edit it later.</p>
          <div class="mono">${esc(cfgPath)}</div>
          <div class="btns">
            <button class="primary" data-first="new">${icon("plus")}Add a device</button>
            <button class="ghost" data-first="import">${icon("download")}Import a list</button>
            <button class="ghost" data-first="cfg">${icon("file-pen-line")}Open config file</button>
          </div>
        </div>`;
  } else if (listMode === "map") {
    listEl.innerHTML = mapHtml();
  }
  paintRows();
  renderDetail();
  treeEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

// How a device is reached, as one glyph. `primary` arrives resolved from Rust, so
// there is no sixth case.
const KIND = {
  ssh: ["square-terminal", "SSH"],
  sftp: ["folder", "Files over SSH"],
  rdp: ["monitor", "Remote desktop"],
  vnc: ["screen-share", "VNC"],
  telnet: ["terminal", "Telnet"],
  serial: ["cable", "Serial console"],
  web: ["globe", "Web UI"],
};

const collate = (a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });
const byName = (a, b) => collate(a.label, b.label);

// One row of the map. `i` indexes `shown`, or is -1 for a device the tree has folded
// away, which is why the row carries its name too. `hub` and `behind` are a row heading
// a branch, and the chip counting what sits behind it.
function jackRow(j, i, { nested = false, indent = nested, hub = false, behind = null } = {}) {
  const state = dotState(j.name);
  const tint = osColor(j.os);
  return `<div class="jack${nested ? " arm" : ""}${hub ? " hub" : ""}" data-i="${i}" data-name="${esc(j.name)}"${
    indent ? ` style="margin-left:18px"` : ""
  }>
    <span class="dot ${state}"></span>
    <span class="os"${j.os ? ` data-tip="${esc(j.os)}"` : ""}${
      tint ? ` style="color:${esc(tint)}"` : ""
    }>${osIcon(j.os)}</span>
    <span class="name">${esc(j.label)}</span>
    <span class="host">${esc(j.user ? j.user + "@" + j.host : j.host)}${j.port ? esc(":" + j.port) : ""}</span>
    <span class="kind" data-tip="${esc((KIND[j.primary] ?? KIND.ssh)[1])}">${icon(
      (KIND[j.primary] ?? KIND.ssh)[0],
    )}</span>
    ${
      j.url && j.primary !== "web"
        ? `<span class="web" data-tip="${esc(j.url)}" data-tip-at="right">${icon("globe")}</span>`
        : ""
    }
    ${behind ?? `<span class="folders">${j.folders.map((f) => `<span class="folder">${esc(f.split("/").pop())}</span>`).join("")}</span>`}
  </div>`;
}

const behindChip = (n, down) =>
  `<span class="behind${down ? " down" : ""}">${icon("share-2")}${n} behind</span>`;

// ── map ────────────────────────────────────────────────────────────────────
// The same rows, grouped by the route to them instead of by folder: a bastion and
// everything behind it are one branch, however they are filed.

// A jack's chain is a path, never a graph, so this is a tree.
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
  const js = all.filter(inGroup);
  const at = (j) => shown.indexOf(j);
  const tree = chainTree(js);

  const walk = (node, depth, blocked) => {
    // A hop that is itself a device heads its branch rather than repeating under it.
    const rows = node.leaves
      .filter((j) => !node.kids.has(j.name))
      .sort(byName)
      .map((j) => jackRow(j, at(j), { nested: depth > 0 }));

    for (const kid of [...node.kids.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      const via = js.find((j) => j.name === kid.name);
      const p = probes.get(kid.name);
      // A bastion that isn't answering explains everything behind it; say so once.
      const down = blocked || (p && p.ms == null);
      const behind = kid.leaves.length + kid.kids.size;
      const chip = behindChip(behind, down);
      // One step off its own .hop; the nesting stacks the indent, so never depth*18.
      rows.push(`<div class="hop ${down ? "blocked" : ""}"${depth ? ` style="margin-left:18px"` : ""}>
        ${
          via
            ? jackRow(via, at(via), { nested: depth > 0, indent: false, hub: true, behind: chip })
            : `<div class="jack hopraw hub${depth > 0 ? " arm" : ""}">
          <span class="dot unknown"></span><span class="os">${icon("waypoints")}</span>
          <span class="name">${esc(kid.name)}</span>
          <span class="host">not in your list</span>
          ${chip}</div>`
        }
        ${walk(kid, depth + 1, down)}
      </div>`);
    }
    return rows.join("");
  };

  const html = walk(tree, 0, false);
  return html || `<p class="empty">nothing here</p>`;
}

function renderDetail() {
  // A pane being typed into is never redrawn: the probe sweep re-renders every
  // thirty seconds, and innerHTML would take a half-written note with it.
  // Only a field: a button keeps focus after a click off macOS, and the detail has
  // to follow the selection all the same.
  const typing = document.activeElement?.matches?.("input, textarea");
  if (
    typing &&
    (detailEl.contains(document.activeElement) || wideEl.contains(document.activeElement))
  )
    return;
  // A live session tab wins: the pane describes what you are typing into.
  const live = activeId !== null ? sessions.get(activeId) : null;
  if (live)
    return renderJack(
      all.find((x) => x.name === live.name),
      live,
    );
  if (detailMode === "group" || !shown[sel]) return renderGroup();
  renderJack(shown[sel], null);
}

// One renderer, two places: wide across the right-hand side while browsing, and the
// narrow pane beside a session.
const wideEl = $("wide");
const dBody = () => (activeId !== null ? detailEl : $("w-body"));
const dActs = () => (activeId !== null ? dActions : $("w-actions"));

const groupLabel = () =>
  !group ? "All devices" : (group.path ?? VAULTS[group.vault]?.[1] ?? "All devices");
// The folder a vault's badge sits beside, wherever a device or a folder is named.
const vaultBadge = (v) =>
  v ? `<span class="d-vault ${esc(v)}">${icon(VAULTS[v][0])}${VAULTS[v][1]}</span>` : "";

// Which tiles the overview shows: "all", "up" or "out". Back to all with every folder picked.
let reachShow = "all";

const tallyOf = (js) => {
  const c = { up: 0, down: 0 };
  for (const j of js) {
    const s = dotState(j.name);
    if (s in c) c[s]++;
  }
  return c;
};

// What is directly in the folder being looked at: its folders, each with how much of it
// answers, then its own devices.
function groupTiles(members) {
  const path = group?.path ?? null;
  const kids = new Map();
  const direct = [];
  for (const j of members) {
    const fs = j.folders.map(normPath).filter(Boolean);
    if (path === null ? !fs.length : fs.includes(path)) direct.push(j);
    for (const f of fs) {
      if (path !== null && !f.startsWith(path + "/")) continue;
      const name = (path === null ? f : f.slice(path.length + 1)).split("/")[0];
      const id = { path: path === null ? name : `${path}/${name}`, vault: j.vault ?? undefined };
      const k = gkey(id);
      if (!kids.has(k)) kids.set(k, { id, name, js: new Set() });
      kids.get(k).js.add(j);
    }
  }
  const dev = (j) => ({
    js: [j],
    name: j.name,
    label: j.label,
    kind: (KIND[j.primary] ?? KIND.ssh)[1],
  });
  return [
    ...[...kids.values()]
      .sort((a, b) => collate(a.name, b.name))
      .map((k) => ({
        js: [...k.js],
        id: k.id,
        label: k.name,
        kind: `${k.js.size} device${k.js.size === 1 ? "" : "s"}`,
      })),
    ...direct.map(dev),
  ];
}

// One dot for several devices, and the words for it: a tile's, and a folder's row.
function reachSaid(js) {
  const n = js.length;
  const { up, down } = tallyOf(js);
  // "Out of reach", not "down": most of a list only answers from a network this
  // machine is not on, and that is not a fault.
  return prefs.probe === false
    ? ["unknown", "Not checked", up]
    : up === n
      ? ["up", n === 1 ? "Reachable" : "All reachable", up]
      : up
        ? ["up", `${up} of ${n} reachable`, up]
        : down < n
          ? ["checking", "Still checking", up]
          : ["", "Out of reach", up];
}

function tileHtml(t) {
  const n = t.js.length;
  const [dot, say, up] = reachSaid(t.js);
  const tint = t.name && osColor(t.js[0].os);
  return `<button type="button" class="tile" ${t.name ? `data-name="${esc(t.name)}"` : idAttrs(t.id)}>
    <span class="tile-top"><span class="os"${tint ? ` style="color:${esc(tint)}"` : ""}>${
      t.name ? osIcon(t.js[0].os) : icon("folder")
    }</span><b>${esc(t.label)}</b><small>${esc(t.kind)}</small></span>
    ${n > 1 && prefs.probe !== false ? `<span class="meter"><i style="width:${(up / n) * 100}%"></i></span>` : ""}
    <span class="tile-say"><i class="dot ${dot}"></i>${say}</span>
  </button>`;
}

function renderGroup() {
  const members = all.filter(inGroup);
  const { up, down } = tallyOf(members);
  const rest = members.length - up - down;
  const probing = prefs.probe !== false;
  const open = [...sessions.values()].filter(
    (s) => !s.dead && members.some((j) => j.name === s.name),
  );
  // Yours first, then whoever else the shared list says is on something in here.
  const on = [
    ...open.map((s) => ["You", labelOf(s.name)]),
    ...members.flatMap((j) => (present.get(j.name) ?? []).map((who) => [who, j.label])),
  ];
  // Every device, or a whole vault, has no folder to rename or delete.
  const real = !!group && group.path !== null;
  const gk = gkey(group);
  const tiles = groupTiles(members).filter((t) => {
    const c = tallyOf(t.js);
    // The chips are not drawn with checks off, so a filter left on could not be undone.
    return !probing || reachShow === "all" || (reachShow === "up" ? c.up > 0 : c.down > 0);
  });
  const chip = (id, label, n) =>
    `<button type="button" data-reach="${id}" aria-pressed="${reachShow === id}">${label}<small>${n}</small></button>`;

  dBody().innerHTML = `
    <div class="d-name">${esc(groupLabel())}${real ? vaultBadge(group.vault) : ""}</div>
    <div class="d-desc">${members.length} device${members.length === 1 ? "" : "s"}${
      real && group.path.includes("/")
        ? ` · in ${esc(group.path.slice(0, group.path.lastIndexOf("/")))}`
        : ""
    }${!real && group?.vault === "team" && vaults.team ? ` · ${esc(vaults.team)}` : ""}</div>

    <div class="d-cards full">
    ${
      !members.length
        ? ""
        : `<section class="d-card">
    <div class="d-sec">${icon("plug")}Reachable from here</div>
    ${
      probing
        ? `<div class="reach"><b>${up}</b>of ${members.length} answer${up === 1 ? "s" : ""} from this network${
            rest ? ` · ${rest} still checking` : ""
          }</div>
    <span class="meter"><i style="width:${(up / members.length) * 100}%"></i></span>
    ${down ? `<p class="page-note">The rest may sit behind a VPN or a network this machine isn't on.</p>` : ""}
    <div class="chips">${chip("all", "All", members.length)}${chip("up", "Reachable", up)}${chip("out", "Out of reach", down)}</div>`
        : `<p class="page-note">Reachability checks are off.</p>`
    }
    </section>`
    }
    ${
      real
        ? `<section class="d-card d-notes"><div class="d-sec">${icon("file-pen-line")}Notes</div>
      <textarea class="d-note" id="gnote" rows="4" spellcheck="false"
        placeholder="What somebody arriving here needs to know."></textarea></section>
      <section class="d-card"><div class="d-sec">${icon("settings")}For every device in here</div>
      <p class="page-note">Used by any device in this folder that doesn't set its own. A
        nearer folder wins.</p>
      <div class="grid2" id="gdefs">
        <label class="f"><span>User</span><input data-k="user" spellcheck="false" placeholder="none" /></label>
        <label class="f"><span>SSH port</span><input data-k="port" inputmode="numeric" placeholder="22" /></label>
        <label class="f"><span>Private key file</span><input data-k="key" spellcheck="false" list="sshkeys" placeholder="none" /></label>
        <label class="f"><span>Jump host</span><input data-k="jump" spellcheck="false" list="jacknames" placeholder="none" /></label>
      </div></section>`
        : ""
    }
    ${
      on.length
        ? `<section class="d-card"><div class="d-sec">${icon("users")}On now</div>
      <div class="on-now">${on.map(([who, what]) => `<span><b>${esc(who)}</b><small>${esc(what)}</small></span>`).join("")}</div></section>`
        : ""
    }
    ${
      !members.length
        ? ""
        : `<section class="d-tiles"><div class="d-sec">${icon("folder")}In here</div>
      ${tiles.length ? `<div class="tiles">${tiles.map(tileHtml).join("")}</div>` : `<p class="page-note">Nothing in here matches.</p>`}</section>`
    }
    </div>
`;

  // Set as a value, not interpolated: a `</textarea>` in a note would end the element.
  if (real) {
    // One at a time: a second blur before the last save's reload has landed would
    // send the stamp that save just moved.
    let saving = Promise.resolve();
    const queue = (work) => (saving = saving.then(work).catch(alertish));
    const vault = group.vault ?? null;

    // Values set rather than interpolated, like the note: a colleague wrote them.
    const lent = folderDefs.get(gk) ?? {};
    const fields = [...$("gdefs").querySelectorAll("input")];
    for (const f of fields) f.value = lent[f.dataset.k] ?? "";
    // On change, which is a blur for a text field: each save rewrites the whole file.
    $("gdefs").addEventListener("change", () => {
      const path = group.path;
      const next = Object.fromEntries(fields.map((f) => [f.dataset.k, f.value.trim() || null]));
      if (next.port !== null) {
        const port = Number(next.port);
        if (!Number.isInteger(port) || port < 1 || port > 65535) {
          return alertish(`"${next.port}" isn't a port`);
        }
        next.port = port;
      }
      queue(async () => {
        try {
          await invoke("save_folder_defaults", {
            path,
            defaults: next,
            stamp: noteStamps.get(gk) ?? null,
            vault,
          });
        } catch (e) {
          alertish(e);
        }
        // Either way: a refusal means someone else's values, and they belong in the
        // fields before the next change saves over them.
        await load();
      });
    });

    const box = $("gnote");
    box.value = notes.get(gk) ?? "";
    // On blur, not per keystroke: each save rewrites the whole config file.
    box.addEventListener("blur", () => {
      const path = group.path;
      const note = box.value;
      queue(async () => {
        if (note === (notes.get(gk) ?? "")) return;
        try {
          await invoke("save_note", { path, note, stamp: noteStamps.get(gk) ?? null, vault });
          notes.set(gk, note.trim());
          // The stamp moved with the write; a second edit must not refuse itself.
          await load();
        } catch (e) {
          alertish(e);
          // Refused as someone else's: their note is now in the pane, and the next
          // blur saves against it.
          await load();
        }
      });
    });
  }

  dActs().innerHTML = real
    ? `<button class="ghost" data-gact="rename" data-tip="Rename folder">${icon("pencil")}</button>
    <button class="ghost danger" data-gact="del" data-tip="Delete folder" data-tip-at="right">${icon("trash-2")}</button>`
    : "";
}

// A team device's host key: what the team has seen, and whether this machine agrees.
// Asked for once per device, when it is first looked at.
function hostKeyRow(j) {
  if (j.vault !== "team" || !j.ssh) return "";
  if (!hostKeys.has(j.name)) {
    hostKeys.set(j.name, null);
    invoke("host_key", { name: j.name })
      .then((k) => {
        hostKeys.set(j.name, k);
        renderDetail();
      })
      .catch(() => {});
  }
  const k = hostKeys.get(j.name);
  if (!k || (!k.team.length && !k.mine.length)) return "";
  // The fingerprint on a line of its own, and anything to say or do about it under it.
  const said = k.team.length
    ? `<span class="d-print">${esc(k.team[0])}<span class="d-from" data-tip="Recorded beside the shared list: ssh checks against it for everyone">team</span></span>${
        k.differs
          ? `<span class="d-warn">This machine has a different key for it. One of the two is not the machine the other saw.</span>`
          : ""
      }`
    : `<span class="d-dim">Only this machine has seen its key.</span>
       <button class="ghost" data-act="vouch" data-tip="Record the key this machine trusts, so colleagues are checked against it">${icon("users")}Share with the team</button>`;
  return `<div class="d-row"><dt>id</dt><dd class="d-key">${said}</dd></div>`;
}

// A value the device didn't set itself, and who lent it. Nothing for a device's own:
// the marker is there to answer "why does it say that", not to label every row.
function lentFrom(j, key) {
  const who = j.from?.[key];
  if (!who) return "";
  const label = who === "defaults" ? "defaults" : who.split("/").join(" / ");
  return `<span class="d-from" data-tip="Not set on this device">${esc(label)}</span>`;
}

function renderJack(j, live) {
  if (!j) {
    dBody().innerHTML = "";
    dActs().innerHTML = "";
    return;
  }
  const p = probes.get(j.name);
  const said = dotState(j.name);
  const reach =
    said === "unknown"
      ? `<span style="color:var(--fg-faint)">not checked</span>`
      : said === "checking"
        ? `<span style="color:var(--fg-faint)">checking…</span>`
        : said === "down"
          ? `<span style="color:var(--down)">no answer</span> · ${esc(p.target)}`
          : // Up by its session: the sweep may not have come round, or may have got no answer.
            p?.ms == null
            ? `<span style="color:var(--up)">up</span> · a session is open`
            : `<span style="color:var(--up)">up</span> · ${esc(p.target)} · ${esc(p.ms)}ms`;

  const stops = [...j.hops, j.user ? `${j.user}@${j.host}` : j.host];
  const mine = tunnels.filter((t) => t.jack === j.name);
  dBody().innerHTML = `
    <div class="d-name">${esc(j.label)}${vaultBadge(j.vault)}</div>
    ${j.desc ? `<div class="d-desc">${esc(j.desc)}</div>` : `<div class="d-desc"></div>`}

    <div class="d-cards">
    <section class="d-card">
    <div class="d-sec">${icon("server")}Target</div>
    <dl>
      <div class="d-row"><dt>host</dt><dd>${esc(j.host)}</dd></div>
      ${j.user ? `<div class="d-row"><dt>user</dt><dd>${esc(j.user)}${lentFrom(j, "user")}</dd></div>` : ""}
      ${j.port ? `<div class="d-row"><dt>port</dt><dd>${esc(j.port)}${lentFrom(j, "port")}</dd></div>` : ""}
      ${j.key ? `<div class="d-row"><dt>key</dt><dd>${esc(j.key)}${lentFrom(j, "key")}</dd></div>` : ""}
      ${j.url ? `<div class="d-row"><dt>web</dt><dd>${esc(j.url)}</dd></div>` : ""}
      ${j.rdp ? `<div class="d-row"><dt>rdp</dt><dd>${esc(j.rdp)}</dd></div>` : ""}
      ${hostKeyRow(j)}
      ${
        j.folders.length
          ? `<div class="d-row"><dt>in</dt><dd>${j.folders
              .map((f) => `<span class="d-in">${esc(f.split("/").join(" / "))}</span>`)
              .join("")}</dd></div>`
          : ""
      }
    </dl>
    </section>
    ${
      mine.length
        ? `<section class="d-card"><div class="d-sec">${icon("waypoints")}Tunnel</div>
      <div class="route">${mine
        .map(
          (t) => `<span class="last"><i class="pip"></i>${
            /* A -R binds on the far end, so there is no local address to print. */
            t.local ? `127.0.0.1:${esc(t.local)}` : "held open on the far end"
          }
        <i class="arm">via ${esc(t.via)}</i></span>`,
        )
        .join("")}</div>
      <div class="btns"><button class="ghost danger" data-act="untunnel"
        data-tip="Close the forward">${icon("unplug")}Close tunnel</button></div></section>`
        : ""
    }

    <section class="d-card">
    <div class="d-sec">${icon("plug")}Reachable</div>
    <div style="font-size:12.5px">${reach}</div>
    ${
      present.has(j.name) ? `<div class="d-here">${icon("users")}${esc(hereTip(j.name))}</div>` : ""
    }
    <div class="btns">
      ${
        j.hops.length
          ? `<button class="ghost" data-act="break" data-tip="Ask each hop whether the next one answers"${
              breaks.get(j.name)?.busy ? " disabled" : ""
            }>${icon("search")}${breaks.get(j.name)?.busy ? "Asking…" : "Find the break"}</button>`
          : ""
      }
      ${
        // A console cable has no address to ping.
        j.host
          ? `<button class="ghost" data-act="ping" data-tip="${j.hops.length ? `Ping from ${esc(j.hops.at(-1))}` : "Ping this host"}">${icon("plug")}Ping</button>
      <button class="ghost" data-act="trace" data-tip="${j.hops.length ? `Trace from ${esc(j.hops.at(-1))}` : "Trace the route there"}">${icon("waypoints")}Trace</button>`
          : ""
      }
      ${
        j.ssh
          ? `<button class="ghost" data-act="key" data-tip="Make your key if you have none, then put it on this device">${icon("key-round")}Install my key</button>`
          : ""
      }
      ${
        j.mac
          ? `<button class="ghost" data-act="wake" data-tip="Wake on LAN, from this network">${icon("power")}Wake</button>`
          : ""
      }
    </div>
    </section>

    <section class="d-card span"${j.host ? "" : " hidden"}>
    <div class="d-sec">${icon("waypoints")}Route</div>
    <div class="route">
      <span class="here"><i class="pip"></i><em class="hop">this machine</em></span>
      ${stops
        .map((h, i) => {
          // What "Find the break" heard at this stop, if it was asked.
          const heard = breaks.get(j.name)?.steps?.[i];
          const state = heard?.ok === true ? " ok" : heard?.ok === false ? " bad" : "";
          return `<span class="${i === stops.length - 1 ? "last" : ""}${state}">
        <i class="pip"></i><em class="hop" data-tip="${esc(h)}">${esc(h)}</em>${
          i < stops.length - 1 ? `<i class="arm">jump</i>` : ""
        }${heard?.said ? `<i class="arm said">${esc(heard.said)}</i>` : ""}</span>`;
        })
        .join("")}
    </div>
    </section>

    ${
      j.forward.length
        ? `<section class="d-card"><div class="d-sec">${icon("arrow-right-left")}Forwards</div>
      <dl>${j.forward
        .map((f) => {
          // A bare forward is `-L`, as `patchbay::forward_arg` reads it.
          const m = /^(-[LRD])\s+(.+)$/.exec(f.trim());
          return `<div class="d-row"><dt>${m ? m[1] : "-L"}</dt><dd>${esc(m ? m[2] : f)}</dd></div>`;
        })
        .join("")}</dl>
      ${
        mine.length
          ? ""
          : `<div class="btns"><button class="ghost" data-act="forward"
        data-tip="Hold these open without a session">${icon("arrow-right-left")}Open forwards</button></div>`
      }</section>`
        : ""
    }

    ${
      j.ssh
        ? `<section class="d-card span"><div class="d-sec">${icon("square-terminal")}Command</div>
    <div class="d-cmd">
      <div class="mono ${j.command.startsWith("ssh ") ? "" : "err"}">${esc(j.command)}</div>
      <button class="flat d-copy" data-act="copy" data-tip="Copy" data-tip-at="right">${icon("copy")}</button>
    </div></section>`
        : ""
    }
    </div>
`;

  dActs().innerHTML = `
    ${
      live
        ? /* A web tab is a page: closed, not disconnected. Plain: the way out is not
             the thing to press, and the tab has its own. */
          `<button class="ghost" data-act="disconnect">${icon("x")}${
            live.dead || live.kind === "web" ? "Close" : "Disconnect"
          }</button>`
        : j.primary === "rdp"
          ? `<button class="primary" data-act="rdp">${icon("monitor")}Connect</button>`
          : j.primary === "vnc"
            ? `<button class="primary" data-act="vnc">${icon("screen-share")}Connect</button>`
            : j.primary === "telnet" || j.primary === "serial"
              ? `<button class="primary" data-act="${j.primary}">${icon(KIND[j.primary][0])}Connect</button>`
              : j.primary === "web"
                ? `<button class="primary" data-act="web">${icon("globe")}Open</button>`
                : j.primary === "sftp"
                  ? `<button class="primary" data-act="files">${icon("folder")}Browse files</button>`
                  : `<button class="primary" data-act="connect">${icon("square-terminal")}Connect</button>`
    }
    ${j.ssh && j.primary !== "ssh" ? `<button class="ghost" data-act="connect" data-tip="Connect over ssh">${icon("square-terminal")}</button>` : ""}
    ${j.ssh && j.primary !== "sftp" ? `<button class="ghost" data-act="files" data-tip="Browse files over sftp">${icon("folder")}</button>` : ""}
    ${j.url && j.primary !== "web" ? `<button class="ghost" data-act="web" data-tip="Open web UI">${icon("globe")}</button>` : ""}
    ${j.rdp && j.primary !== "rdp" ? `<button class="ghost" data-act="rdp" data-tip="Remote desktop">${icon("monitor")}</button>` : ""}
    ${j.vnc && j.primary !== "vnc" ? `<button class="ghost" data-act="vnc" data-tip="VNC">${icon("screen-share")}</button>` : ""}
    ${j.telnet && j.primary !== "telnet" ? `<button class="ghost" data-act="telnet" data-tip="Telnet">${icon("terminal")}</button>` : ""}
    ${j.serial && j.primary !== "serial" ? `<button class="ghost" data-act="serial" data-tip="Serial console">${icon("cable")}</button>` : ""}
    <button class="ghost" data-act="edit" data-tip="Edit device">${icon("pencil")}</button>
    <button class="ghost" data-act="dup" data-tip="Duplicate device">${icon("copy-plus")}</button>
    ${
      j.vault && !live
        ? `<button class="ghost" data-act="move" data-tip="${
            j.vault === "mine"
              ? "Everyone on the list will see it"
              : "Only this machine will see it"
          }">${icon(VAULTS[otherVault(j.vault)][0])}Move to ${VAULTS[otherVault(j.vault)][1]}</button>`
        : ""
    }`;
}

// Out of a session and back to the tree's own view: what a click in the sidebar means.
function browse() {
  if (activeId === null) return;
  activeId = null;
  showTab();
  renderTabs();
}

// Selection must not rebuild the tree: replacing innerHTML destroys the row under
// the cursor, so two clicks never pair into a dblclick.
// `stay` is a right-click: the row is picked for its menu, and the session in front
// stays in front.
function select(i, stay = false) {
  if (!shown.length) return;
  detailMode = "jack";
  // A device is where the keyboard is now; a folder mark left behind would take its ⌫.
  unmarkFolders();
  sel = (i + shown.length) % shown.length;
  selKey = keyAt(sel);
  // Asked again each time a device is picked: a first connect, or a colleague's share,
  // changes the answer while the window is open.
  hostKeys.delete(shown[sel].name);
  // The folder it was picked in is the one "New device" and a move start from.
  // Not on the map, which is drawn from the folder it was opened on.
  if (listMode !== "map") group = gkey(shownIn[sel]) ? shownIn[sel] : null;
  if (!stay) browse();
  paintRows();
  renderDetail();
  treeEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

// A device by name, for a row the tree has folded away (the map draws those too):
// its folder is opened, and the answer is where it now is in `shown`.
function revealDevice(name) {
  const j = all.find((x) => x.name === name);
  const parts = normPath(j?.folders[0] ?? "")
    .split("/")
    .filter(Boolean);
  for (let n = 1; n <= parts.length; n++) {
    openGroup(gkey({ path: parts.slice(0, n).join("/"), vault: j.vault ?? undefined }));
  }
  render();
  return shown.findIndex((x) => x.name === name);
}

// Selection, marks and the current folder, painted in place. Keyed on each row's
// `data-i`, not its position: the map reorders rows, and a jump host not in the list
// has no device.
function paintRows() {
  for (const el of document.querySelectorAll("#tree .jack[data-i], #list .jack[data-i]")) {
    const j = shown[+el.dataset.i];
    el.setAttribute("aria-selected", detailMode === "jack" && +el.dataset.i === sel);
    el.classList.toggle("marked", marked.has(j?.name));
  }
  for (const el of treeEl.querySelectorAll(".group")) {
    const k = gkey(idOf(el));
    el.setAttribute("aria-current", detailMode === "group" && !!group && k === gkey(group));
    el.classList.toggle("marked", markedFolders.has(k));
  }
  // The arrow keys move a selection, not the focus, so the tree tells a screen reader
  // which row that is. One id, moved to whichever row is current.
  document.getElementById("tree-active")?.removeAttribute("id");
  const on = treeEl.querySelector('[aria-selected="true"], [aria-current="true"]');
  if (on) {
    on.id = "tree-active";
    treeEl.setAttribute("aria-activedescendant", "tree-active");
  } else treeEl.removeAttribute("aria-activedescendant");
  renderDock();
}

// The bulk-action dock, shown when two or more rows are marked. Same actions as the
// context menu, so the gesture is discoverable without knowing the chord.
function renderDock() {
  const dock = $("dock");
  if (!dock) return;
  const btn = (a, ic, lbl, extra = "") =>
    `<button type="button" class="ghost" data-a="${esc(a)}"${extra}>${icon(ic)}${esc(lbl)}</button>`;
  // Guarded: this runs on every arrow key, and foldersMarked walks every device.
  const fs = markedFolders.size > 1 ? foldersMarked() : [];
  if (fs.length > 1) {
    dock.innerHTML = `
    <span class="count"><b>${fs.length}</b> folders selected</span>
    ${btn("delfolders", "trash-2", `Remove ${fs.length}`, ' data-danger="1"')}`;
    dock.hidden = false;
    return;
  }
  const bulk = markedHere();
  if (bulk.length < 2) {
    dock.hidden = true;
    dock.innerHTML = "";
    return;
  }
  const ssh = bulk.filter((j) => j.ssh).length;
  dock.innerHTML = `
    <span class="count"><b>${bulk.length}</b> selected</span>
    ${ssh >= 2 ? btn("bcast", "radio-tower", `Broadcast to ${ssh}`) : ""}
    ${btn("move", "folder-input", "Move")}
    ${btn("del", "trash-2", `Delete ${bulk.length}`, ' data-danger="1"')}`;
  dock.hidden = false;
}

$("dock")?.addEventListener("click", (e) => {
  const el = e.target.closest("[data-a]");
  const a = el?.dataset.a;
  if (!a) return;
  if (a === "delfolders") return removeFolders(foldersMarked());
  const bulk = markedHere();
  if (!bulk.length) return;
  if (a === "bcast") return openBroadcast(bulk);
  if (a === "move") return moveAsked(bulk);
  if (a === "del") return removeMarked(bulk);
});

// ⌘-click marks a row; shift-click marks the run from `sel`. `sel` stays put as the
// anchor. Marks are names, the selection is an index: the list is refiltered under
// them on every render.
function markToggle(i) {
  const n = shown[i]?.name;
  if (!n) return;
  // The dock is part of the browse view, and a web tab would paint over it.
  browse();
  unmarkFolders();
  // The first ⌘-click also marks the selected row, or three rows are lit and the
  // dock says two.
  if (!marked.size && detailMode === "jack" && i !== sel && shown[sel]) marked.add(shown[sel].name);
  marked.has(n) ? marked.delete(n) : marked.add(n);
  paintRows();
}
function markRange(i) {
  browse();
  unmarkFolders();
  // With a folder selected there is no row to run from: the range is the click.
  const from = detailMode === "jack" ? sel : i;
  for (let k = Math.min(from, i); k <= Math.max(from, i); k++) marked.add(shown[k].name);
  paintRows();
}

// ⌘A: every device the tree is showing.
function markAll() {
  unmarkFolders();
  marked = new Set(shown.map((j) => j.name));
  paintRows();
}

// What a bulk action applies to: a mark folded out of view is not part of the ask.
// Once each, though a device filed twice is drawn twice.
const markedHere = () => [...new Set(shown.filter((j) => marked.has(j.name)))];

// The same two gestures on folders, anchored on the folder being looked at. A range
// is the rows as drawn, so a folded branch contributes only its own row. Marks stay
// inside one vault: a Delete across both would be two different questions.
function markFolder(id) {
  marked.clear();
  if ([...markedFolders.values()].some((m) => m.vault !== id.vault)) markedFolders.clear();
  if (!markedFolders.size && group?.path && group.vault === id.vault && gkey(group) !== gkey(id))
    markedFolders.set(gkey(group), group);
  markedFolders.has(gkey(id)) ? markedFolders.delete(gkey(id)) : markedFolders.set(gkey(id), id);
  paintRows();
}
function markFolderRange(id) {
  marked.clear();
  const rows = [...treeEl.querySelectorAll('.group[data-group="1"]:not(.vault)')]
    .map(idOf)
    .filter((r) => r.vault === id.vault);
  const keys = rows.map(gkey);
  const from = keys.indexOf(gkey(group));
  const to = keys.indexOf(gkey(id));
  for (const r of rows.slice(Math.min(from < 0 ? to : from, to), Math.max(from, to) + 1))
    markedFolders.set(gkey(r), r);
  paintRows();
}
function unmarkFolders() {
  if (!markedFolders.size) return;
  markedFolders.clear();
  paintRows();
}
// A mark on a folder that has since gone (renamed elsewhere, emptied) is dropped.
const foldersMarked = () =>
  [...markedFolders.values()].filter((id) => folderPaths(id.vault).has(id.path));

// From a folder, Down is the first device drawn and Up the last.
const move = (d) => select(detailMode === "jack" ? sel + d : d > 0 ? 0 : -1);

// A folder row taken as the list: the click and the arrow keys land here.
function pickGroup(id, foldable) {
  group = id;
  reachShow = "all";
  if (foldable) openGroup(gkey(id));
  sel = 0;
  selKey = null;
  marked.clear();
  markedFolders.clear();
  detailMode = "group"; // the detail describes the folder, not its first device
  browse();
  render();
}

// Left folds the current folder or steps up a row; Right unfolds it or steps down.
// From a device, Left is its folder.
function stepTree(d) {
  if (detailMode === "jack" && shown[sel]) {
    if (d < 0 && group) pickGroup(group, false);
    return;
  }
  const rows = [...treeEl.querySelectorAll(".group")];
  const at = rows.findIndex((r) => r.getAttribute("aria-current") === "true");
  const cur = rows[at];
  const id = cur ? idOf(cur) : null;
  const foldable = cur?.dataset.hasKids === "true";
  const open = foldable && expanded.has(gkey(id));
  if (foldable && d < 0 === open) {
    toggleGroup(gkey(id));
    return render();
  }
  const next = rows[at + d];
  if (next) pickGroup(idOf(next), false);
}

// In a tab. "Open in browser" stays on the context menu: a webview has no
// certificate click-through.
async function openWeb(name) {
  used(name);
  await openWebSession(name);
}

// In a tab. The context menu still offers the handoff to the system client.
async function openRdp(name) {
  used(name);
  await openRdpSession(name);
  await refreshTunnels();
}

// In a tab. The context menu still offers the system's viewer.
async function openVnc(name) {
  used(name);
  await openVncSession(name);
  await refreshTunnels();
}

// Telnet or a serial console: a terminal tab running the system's own tool.
async function openLine(name, via) {
  used(name);
  await openSession(name, null, null, false, via);
  if (via === "telnet") await refreshTunnels();
}

// The OS opens whatever registered `vnc://`.
async function handOffVnc(name) {
  used(name);
  try {
    await invoke("open_vnc", { name });
    await refreshTunnels();
  } catch (e) {
    alertish(e);
  }
}

async function handOffRdp(name) {
  used(name);
  try {
    await invoke("open_rdp", { name });
    await refreshTunnels();
  } catch (e) {
    alertish(e);
  }
}

// A tunnel that died since the last ask is named in a pill: its port stopped answering
// without anything on screen changing.
function takeTunnels(t) {
  tunnels = t.live;
  for (const why of t.ended) alertish(why);
}

async function refreshTunnels() {
  try {
    takeTunnels(await invoke("tunnels"));
    render();
  } catch {
    /* none is normal */
  }
}

// What Enter, a double-click and the palette do.
function primary(name) {
  const j = all.find((x) => x.name === name);
  if (!j) return;
  if (j.primary === "rdp") return openRdp(name);
  if (j.primary === "vnc") return openVnc(name);
  if (j.primary === "telnet" || j.primary === "serial") return openLine(name, j.primary);
  if (j.primary === "web") return openWeb(name);
  if (j.primary === "sftp") return openFilesSession(name);
  return connect(name);
}

// In-app unless the preference says otherwise; `inTerminal` forces the handoff.
async function connect(name, inTerminal = prefs.connect_in_terminal === true) {
  used(name);
  if (!inTerminal) return openSession(name);
  try {
    await invoke("connect", { name });
  } catch (e) {
    alertish(e);
  }
}

// ── command palette ────────────────────────────────────────────────────────
const palOpen = () => !paletteEl.hidden;
// `seed` carries the keystroke that opened the palette from the list.
// `beside` is ⌘D: what is picked opens beside the tab in front instead of in a tab.
let palBeside = false;
function openPalette(seed = "", beside = false) {
  palBeside = beside;
  pq.placeholder = beside ? "Open beside…" : "Connect to…";
  paletteEl.hidden = false;
  pq.value = seed;
  palSel = 0;
  $("pq-icon").innerHTML = icon("search");
  renderPalette();
  pq.focus();
  pq.setSelectionRange(seed.length, seed.length);
}
function closePalette() {
  paletteEl.hidden = true;
}

// Recency first, file order behind it (stable sort). Only the palette ranks by
// recency; the list stays in file order. A folder is a `{ path }`, the selection's
// shape, and folders lead: typing a customer's name is asking for their folder, and a
// device typed in full is one arrow key away.
function palMatches() {
  const f = pq.value.trim().toLowerCase();
  const rank = (j) => {
    const i = recent.indexOf(j.name);
    return i < 0 ? recent.length : i;
  };
  const jacks = all.filter((j) => hit(j, f)).sort((a, b) => rank(a) - rank(b));
  const named = jacks.filter((j) => j.label.toLowerCase().includes(f));
  const folders = f
    ? folderIds()
        .filter((id) => id.path.toLowerCase().includes(f))
        .sort((a, b) => collate(a.path, b.path))
    : [];
  // Only with a shell in front: a snippet is typed, and there has to be a prompt.
  const typed =
    shellInFront() && !palBeside
      ? snips
          .filter((s) => !f || s.name.toLowerCase().includes(f) || s.text.toLowerCase().includes(f))
          .map((s) => ({ snip: s }))
      : [];
  return [...typed, ...folders, ...named, ...jacks.filter((j) => !named.includes(j))].slice(0, 40);
}
function palPick(p) {
  closePalette();
  if (p.snip) return typeSnippet(p.snip);
  if (!p.path) return palBeside ? openBeside(p.name) : primary(p.name);
  const parts = p.path.split("/");
  for (let i = 1; i < parts.length; i++) {
    openGroup(gkey({ path: parts.slice(0, i).join("/"), vault: p.vault }));
  }
  pickGroup(p, true);
}
// A quick connect when nothing matches: `user@host` or `host:2222`, one word. Rust
// applies the same guard a configured host gets.
const adhoc = () => {
  const q = pq.value.trim();
  return q && !/\s/.test(q) && !q.startsWith("-") && !palMatches().length ? q : null;
};
function renderPalette() {
  const rows = palMatches();
  palSel = Math.min(palSel, Math.max(0, rows.length - 1));
  const quick = adhoc();
  // Snippets and folders lead, so the first device is where the two halves meet.
  const cutAt = rows.findIndex((r) => !r.path && !r.snip);
  presultsEl.innerHTML = quick
    ? `<div class="jack" data-adhoc="1" role="option" aria-selected="true">
        <span class="os">${icon("square-terminal")}</span>
        <span class="name">${esc(quick)}</span>
        <span class="host">quick connect</span></div>`
    : rows.length
      ? rows
          .map((j, i) => {
            const cut = i === cutAt && i > 0 ? " cut" : "";
            // The text itself, not only its name: it is about to be on your prompt,
            // and in a shared list someone else may have written it.
            if (j.snip) {
              return `<div class="jack" data-pi="${i}" role="option" aria-selected="${i === palSel}">
        <span class="os">${icon("text-cursor-input")}</span>
        <span class="name">${esc(j.snip.name)}</span>
        <span class="host snip">${esc(j.snip.text)}</span>${
          j.snip.vault ? `<span class="pvault">${VAULTS[j.snip.vault][1]}</span>` : ""
        }</div>`;
            }
            if (j.path) {
              const at = j.path.lastIndexOf("/");
              return `<div class="jack" data-pi="${i}" role="option" aria-selected="${i === palSel}">
        <span class="os">${icon("folder")}</span>
        <span class="name">${esc(j.path.slice(at + 1))}</span>
        <span class="host">${esc(
          [VAULTS[j.vault]?.[1], at < 0 ? "folder" : j.path.slice(0, at)]
            .filter(Boolean)
            .join(" · "),
        )}</span></div>`;
            }
            // Same rule as the list: no brand colour on the selected row.
            const tint = i === palSel ? null : osColor(j.os);
            return `<div class="jack${cut}" data-pi="${i}" role="option" aria-selected="${i === palSel}">
        <span class="os"${tint ? ` style="color:${esc(tint)}"` : ""}>${osIcon(j.os)}</span>
        <span class="name">${esc(j.label)}</span>
        <span class="host">${esc(j.host)}${j.vault === "mine" ? " · Mine" : ""}</span></div>`;
          })
          .join("")
      : `<p class="empty">no match</p>`;
  presultsEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}
