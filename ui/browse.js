// The device browser: folder tree, list, detail pane, command palette.
// Classic script sharing one global scope; see the load order in ui/index.html.

// ── sidebar tree ───────────────────────────────────────────────────────────
// "prod/eu/web" nests three deep; a jack counts toward every ancestor and every
// branch it is filed in.
function buildTree(jacks) {
  const root = new Map();
  const placed = jacks.flatMap((j) => j.folders.map((f) => [j, f]));
  for (const p of pending.values()) placed.push([null, p.path]);
  for (const [j, folder] of placed) {
    {
      let level = root,
        path = "";
      for (const part of folder
        .split("/")
        .map((p) => p.trim())
        .filter(Boolean)) {
        path = path ? `${path}/${part}` : part;
        if (!level.has(part))
          level.set(part, { name: part, path, members: new Set(), children: new Map() });
        const node = level.get(part);
        if (j) node.members.add(j.name);
        level = node.children;
      }
    }
  }
  return root;
}

// Every folder there is, parents included, for "Move to…" to offer.
function folderPaths() {
  const out = new Set();
  for (const f of [...all.flatMap((j) => j.folders), ...[...pending.values()].map((p) => p.path)]) {
    const parts = f.split("/").filter(Boolean);
    for (let i = 1; i <= parts.length; i++) out.add(parts.slice(0, i).join("/"));
  }
  return out;
}

function renderTree() {
  const live = new Set([...sessions.values()].filter((s) => !s.dead).map((s) => s.name));
  const names = (js) => new Set(js.map((j) => j.name));
  const rows = [
    row({ name: "All jacks", members: names(all), children: new Map() }, 0, "layers", live, null),
  ];

  const walk = (level, depth) => {
    for (const node of [...level.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      rows.push(row(node, depth, undefined, live, { path: node.path }));
      if (expanded.has(gkey({ path: node.path }))) walk(node.children, depth + 1);
    }
  };

  // Nested folders sort ahead of flat ones so a hierarchy isn't buried among them.
  const tree = buildTree(all);
  const roots = [...tree.values()].sort(
    (a, b) => (b.children.size > 0) - (a.children.size > 0) || a.name.localeCompare(b.name),
  );
  if (roots.length) rows.push(`<div class="tree-sep"></div>`);
  for (const node of roots) {
    rows.push(row(node, 0, undefined, live, { path: node.path }));
    if (expanded.has(gkey({ path: node.path }))) walk(node.children, 1);
  }

  treeEl.innerHTML = rows.join("");
}

function row(node, depth, glyph, live, id) {
  const kids = node.children.size > 0;
  const open = expanded.has(gkey(id));
  const g = glyph ?? (kids && open ? "folder-open" : "folder");
  // The dot is always in the layout for its auto margin; painted only with a live session.
  const on = live && [...node.members].some((n) => live.has(n));
  return `<div class="group" data-path="${esc(id?.path ?? "")}"
       data-group="${id ? "1" : ""}" data-has-kids="${kids}"
       aria-current="${sameGroup(group, id)}" style="padding-left:${8 + depth * 13}px">
    <span class="twist ${kids ? "" : "leaf"} ${open ? "open" : ""}">${icon("chevron-right")}</span>
    <span class="gi">${icon(g)}</span>
    <span class="label">${esc(node.name)}</span>
    <span class="live ${on ? "on" : ""}"${on ? ' data-tip="A session is open in here"' : ""}></span>
    <span class="n">${node.members.size}</span>
  </div>`;
}

// What the status dot says. "checking" waits on a running sweep; "unknown" means
// reachability checks are off. One decider, so the dot, the pane and the tally agree.
function dotState(name) {
  if (prefs.probe === false) return "unknown";
  const p = probes.get(name);
  return !p ? "checking" : p.ms == null ? "down" : "up";
}

const inGroup = (j) =>
  group === null
    ? true
    : group.path === null
      ? true
      : j.folders.some((f) => f === group.path || f.startsWith(group.path + "/"));

// ── list ───────────────────────────────────────────────────────────────────
function render() {
  // Not while a press is in flight: `innerHTML` takes away the row or button being
  // pressed, and the click that would have fired on it never does. `releasePress()`
  // runs the skipped render once the click has landed.
  if (pressing) {
    missedRender = true;
    return;
  }
  renderTree();
  shown = all.filter(inGroup);
  // Both the device sheet and the settings sheet suggest jump targets.
  $("jacknames").innerHTML = all.map((x) => `<option value="${esc(x.name)}">`).join("");
  $("sshkeys").innerHTML = sshKeys.map((k) => `<option value="${esc(k)}">`).join("");
  $("folderlist").innerHTML = [...folderPaths()]
    .sort()
    .map((f) => `<option value="${esc(f)}">`)
    .join("");

  renderTabs();
  searchBtn.innerHTML = `${icon("search")}Search<kbd>${chord("k")}</kbd>`;
  $("viewmode").innerHTML = listMode === "map" ? `${icon("list")}List` : `${icon("share-2")}Map`;
  $("viewmode").dataset.tip =
    listMode === "map" ? "Back to the flat list" : "Group by the route to each device";
  $("newjack").innerHTML = `${icon("plus")}Device<kbd>${chord("n")}</kbd>`;
  $("newgroup").innerHTML = icon("folder-plus");
  $("newgroup").dataset.tip = "New folder";
  $("editcfg").innerHTML = icon("file-pen-line");
  $("editcfg").dataset.tip = `Open the config file  ${chord("e")}`;
  $("settings").innerHTML = icon("settings");
  $("settings").dataset.tip = `Settings  ${chord(",")}`;
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
            <button class="ghost" data-first="import">${icon("download")}Import a list</button>
            <button class="ghost" data-first="cfg">${icon("file-pen-line")}Open config file</button>
          </div>
        </div>`;
    renderDetail(); // a session tab still has something to describe
    return;
  }
  sel = Math.min(sel, shown.length - 1);
  listEl.innerHTML = listMode === "map" ? mapHtml() : shown.map((j, i) => jackRow(j, i)).join("");
  paintRows();
  renderDetail();
  listEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

// How a device is reached, as one glyph. `primary` arrives resolved from Rust, so
// there is no sixth case.
const KIND = {
  ssh: ["square-terminal", "SSH"],
  sftp: ["folder", "Files over SSH"],
  rdp: ["monitor", "Remote desktop"],
  vnc: ["screen-share", "VNC"],
  web: ["globe", "Web UI"],
};

// One row, the same in the list and the map, so selection, double-click and the
// context menu work in both. `i` indexes `shown`. `hub` and `behind` are the map's:
// a row heading a branch, and the chip counting what sits behind it.
function jackRow(j, i, { nested = false, indent = nested, hub = false, behind = null } = {}) {
  const state = dotState(j.name);
  // A brand colour is tuned against the panel, not the accent block of a selected row.
  const tint = i === sel ? null : osColor(j.os);
  return `<div class="jack${nested ? " arm" : ""}${hub ? " hub" : ""}" data-i="${i}" aria-selected="${i === sel}"${
    indent ? ` style="margin-left:18px"` : ""
  }>
    <span class="dot ${state}"></span>
    <span class="os"${j.os ? ` data-tip="${esc(j.os)}"` : ""}${
      tint ? ` style="color:${esc(tint)}"` : ""
    }>${osIcon(j.os)}</span>
    <span class="name">${esc(j.name)}</span>
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
  const at = (j) => shown.indexOf(j);
  const tree = chainTree(shown);

  const walk = (node, depth, blocked) => {
    // A hop that is itself a device heads its branch rather than repeating under it.
    const rows = node.leaves
      .filter((j) => !node.kids.has(j.name))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((j) => jackRow(j, at(j), { nested: depth > 0 }));

    for (const kid of [...node.kids.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      const via = shown.find((j) => j.name === kid.name);
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
  if (detailEl.contains(document.activeElement)) return;
  // A live session tab wins: the pane describes what you are typing into.
  const live = activeId !== null ? sessions.get(activeId) : null;
  if (live)
    return renderJack(
      all.find((x) => x.name === live.name),
      live,
    );
  if (detailMode === "group") return renderGroup();
  renderJack(shown[sel], null);
}

const groupLabel = () => (group === null ? "All jacks" : group.path);

function renderGroup() {
  const members = all.filter(inGroup);
  const up = members.filter((j) => dotState(j.name) === "up").length;
  const down = members.filter((j) => dotState(j.name) === "down").length;
  const rest = members.length - up - down;
  const waiting = prefs.probe !== false;
  const open = [...sessions.values()].filter(
    (s) => !s.dead && members.some((j) => j.name === s.name),
  );
  // "All jacks" has no folder to rename or delete.
  const real = group !== null && group.path !== null;

  detailEl.innerHTML = `
    <div class="d-name"><span class="d-os">${icon(
      real ? "folder-open" : "layers",
    )}</span>${esc(groupLabel())}</div>
    <div class="d-desc">${members.length} device${members.length === 1 ? "" : "s"}${
      real && group.path.includes("/")
        ? ` · in ${esc(group.path.slice(0, group.path.lastIndexOf("/")))}`
        : ""
    }</div>

    <div class="d-sec">${icon("plug")}Reachable</div>
    <div class="tallies">
      <span><i class="dot up"></i>${up} up</span>
      <span><i class="dot down"></i>${down} down</span>
      ${
        rest
          ? `<span><i class="dot ${waiting ? "checking" : "unknown"}"></i>${
              rest
            } ${waiting ? "still checking" : "not checked"}</span>`
          : ""
      }
    </div>

    ${
      open.length
        ? `<div class="d-sec">${icon("square-terminal")}Sessions</div>
      <div class="route">${open.map((s) => `<span class="last"><i class="pip"></i>${esc(s.name)}</span>`).join("")}</div>`
        : ""
    }

    ${
      !real
        ? ""
        : `<div class="d-sec">${icon("file-pen-line")}Notes</div>
      <textarea class="d-note" id="gnote" rows="4" spellcheck="false"
        placeholder="What somebody arriving here needs to know."></textarea>`
    }
`;

  // Set as a value, not interpolated: a `</textarea>` in a note would end the element.
  if (real) {
    const box = $("gnote");
    box.value = notes.get(group.path) ?? "";
    // On blur, not per keystroke: each save rewrites the whole config file.
    box.addEventListener("blur", async () => {
      const was = notes.get(group.path) ?? "";
      if (box.value === was) return;
      try {
        await invoke("save_note", {
          path: group.path,
          note: box.value,
          stamp: noteStamps.get(group.path) ?? null,
        });
        notes.set(group.path, box.value.trim());
        // The stamp moved with the write; a second edit must not refuse itself.
        await load();
      } catch (e) {
        alertish(e);
        // Refused as someone else's: their note is now in the pane, and the next
        // blur saves against it.
        await load();
      }
    });
  }

  dActions.innerHTML = `
    <button class="primary" data-gact="new">${icon("plus")}Device</button>
    ${
      real
        ? `<button class="ghost" data-gact="rename" data-tip="Rename folder">${icon("pencil")}</button>
    <button class="ghost danger" data-gact="del" data-tip="Delete folder" data-tip-at="right">${icon("trash-2")}</button>`
        : ""
    }
`;
}

function renderJack(j, live) {
  if (!j) {
    detailEl.innerHTML = "";
    dActions.innerHTML = "";
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
          : `<span style="color:var(--up)">up</span> · ${esc(p.target)} · ${esc(p.ms)}ms`;

  const stops = [...j.hops, j.user ? `${j.user}@${j.host}` : j.host];
  const mine = tunnels.filter((t) => t.jack === j.name);
  detailEl.innerHTML = `
    <div class="d-name"><span class="d-os"${
      osColor(j.os) ? ` style="color:${esc(osColor(j.os))}"` : ""
    }>${osIcon(j.os)}</span>${esc(j.name)}</div>
    ${j.desc ? `<div class="d-desc">${esc(j.desc)}</div>` : `<div class="d-desc"></div>`}

    <div class="d-sec">${icon("server")}Target</div>
    <dl>
      <div class="d-row"><dt>host</dt><dd>${esc(j.host)}</dd></div>
      ${j.user ? `<div class="d-row"><dt>user</dt><dd>${esc(j.user)}</dd></div>` : ""}
      ${j.port ? `<div class="d-row"><dt>port</dt><dd>${esc(j.port)}</dd></div>` : ""}
      ${j.key ? `<div class="d-row"><dt>key</dt><dd>${esc(j.key)}</dd></div>` : ""}
      ${j.url ? `<div class="d-row"><dt>web</dt><dd>${esc(j.url)}</dd></div>` : ""}
      ${j.rdp ? `<div class="d-row"><dt>rdp</dt><dd>${esc(j.rdp)}</dd></div>` : ""}
    </dl>
    ${
      mine.length
        ? `<div class="d-sec">${icon("waypoints")}Tunnel</div>
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
        data-tip="Close the forward">${icon("unplug")}Close tunnel</button></div>`
        : ""
    }

    <div class="d-sec">${icon("waypoints")}Route</div>
    <div class="route">
      <span><i class="pip"></i>this machine</span>
      ${stops
        .map(
          (h, i) => `<span class="${i === stops.length - 1 ? "last" : ""}">
        <i class="pip"></i>${esc(h)}${i < stops.length - 1 ? `<i class="arm">jump</i>` : ""}</span>`,
        )
        .join("")}
    </div>

    ${
      j.forward.length
        ? `<div class="d-sec">${icon("arrow-right-left")}Forwards</div>
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
      }`
        : ""
    }

    <div class="d-sec">${icon("plug")}Reachable</div>
    <div style="font-size:12.5px">${reach}</div>
    <div class="btns">
      <button class="ghost" data-act="ping" data-tip="${j.hops.length ? `Ping from ${esc(j.hops.at(-1))}` : "Ping this host"}">${icon("plug")}Ping</button>
      <button class="ghost" data-act="trace" data-tip="${j.hops.length ? `Trace from ${esc(j.hops.at(-1))}` : "Trace the route there"}">${icon("waypoints")}Trace</button>
    </div>

    ${
      j.ssh
        ? `<div class="d-sec">${icon("square-terminal")}Command</div>
    <div class="d-cmd">
      <div class="mono ${j.command.startsWith("ssh ") ? "" : "err"}">${esc(j.command)}</div>
      <button class="flat d-copy" data-act="copy" data-tip="Copy" data-tip-at="right">${icon("copy")}</button>
    </div>`
        : ""
    }
`;

  dActions.innerHTML = `
    ${
      live
        ? /* A web tab is a page: closed, not disconnected. */
          `<button class="primary" data-act="disconnect">${icon("x")}${
            live.dead || live.kind === "web" ? "Close" : "Disconnect"
          }</button>`
        : j.primary === "rdp"
          ? `<button class="primary" data-act="rdp">${icon("monitor")}Connect</button>`
          : j.primary === "vnc"
            ? `<button class="primary" data-act="vnc">${icon("screen-share")}Share screen</button>`
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
    <button class="ghost" data-act="edit" data-tip="Edit device">${icon("pencil")}</button>
    <button class="ghost" data-act="dup" data-tip="Duplicate device">${icon("copy-plus")}</button>`;
}

// Selection must not rebuild the list: replacing innerHTML destroys the row under
// the cursor, so two clicks never pair into a dblclick.
function select(i) {
  detailMode = "jack";
  if (!shown.length) return;
  sel = (i + shown.length) % shown.length;
  paintRows();
  renderDetail();
  listEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

// Selection and marks, painted in place. Keyed on each row's `data-i`, not its
// position: the map reorders rows, and a jump host not in the list has no device.
function paintRows() {
  for (const el of listEl.querySelectorAll(".jack[data-i]")) {
    const j = shown[+el.dataset.i];
    el.setAttribute("aria-selected", +el.dataset.i === sel);
    el.classList.toggle("marked", marked.has(j?.name));
  }
  renderDock();
}

// The bulk-action dock, shown when two or more rows are marked. Same actions as the
// context menu, so the gesture is discoverable without knowing the chord.
function renderDock() {
  const dock = $("dock");
  if (!dock) return;
  const bulk = markedHere();
  if (bulk.length < 2) {
    dock.hidden = true;
    dock.innerHTML = "";
    return;
  }
  const ssh = bulk.filter((j) => j.ssh).length;
  const btn = (a, ic, lbl, extra = "") =>
    `<button type="button" class="ghost" data-a="${esc(a)}"${extra}>${icon(ic)}${esc(lbl)}</button>`;
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
  // The first ⌘-click also marks the selected row, or three rows are lit and the
  // dock says two.
  if (!marked.size && i !== sel && shown[sel]) marked.add(shown[sel].name);
  marked.has(n) ? marked.delete(n) : marked.add(n);
  paintRows();
}
function markRange(i) {
  for (let k = Math.min(sel, i); k <= Math.max(sel, i); k++) marked.add(shown[k].name);
  paintRows();
}

// What a bulk action applies to: a mark filtered out of view is not part of the ask.
const markedHere = () => shown.filter((j) => marked.has(j.name));

const move = (d) => select(sel + d);

// A folder row taken as the list: the click and the arrow keys land here.
function pickGroup(id, foldable) {
  group = id;
  if (foldable) openGroup(gkey(id));
  sel = 0;
  marked.clear();
  detailMode = "group"; // the pane describes the folder, not its first device
  render();
}

// Left folds the current folder or steps up a row; Right unfolds it or steps down.
function stepTree(d) {
  const rows = [...treeEl.querySelectorAll(".group")];
  const at = rows.findIndex((r) => r.getAttribute("aria-current") === "true");
  const cur = rows[at];
  const id = cur?.dataset.group ? { path: cur.dataset.path } : null;
  const open = id && cur.dataset.hasKids === "true" && expanded.has(gkey(id));
  if (id && cur.dataset.hasKids === "true" && d < 0 === !!open) {
    toggleGroup(gkey(id));
    return render();
  }
  const next = rows[at + d];
  if (next) pickGroup(next.dataset.group ? { path: next.dataset.path } : null, false);
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

// Always a handoff: the OS opens whatever registered `vnc://`.
async function openVnc(name) {
  used(name);
  try {
    await invoke("open_vnc", { name });
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
function openPalette(seed = "") {
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
// recency; the list stays in file order.
function palMatches() {
  const f = pq.value.trim().toLowerCase();
  const rank = (j) => {
    const i = recent.indexOf(j.name);
    return i < 0 ? recent.length : i;
  };
  return all
    .filter((j) => hit(j, f))
    .sort((a, b) => rank(a) - rank(b))
    .slice(0, 40);
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
  presultsEl.innerHTML = quick
    ? `<div class="jack" data-adhoc="1" aria-selected="true">
        <span class="os">${icon("square-terminal")}</span>
        <span class="name">${esc(quick)}</span>
        <span class="host">quick connect</span></div>`
    : rows.length
      ? rows
          .map((j, i) => {
            // Same rule as the list: no brand colour on the selected row.
            const tint = i === palSel ? null : osColor(j.os);
            return `<div class="jack" data-pi="${i}" aria-selected="${i === palSel}">
        <span class="os"${tint ? ` style="color:${esc(tint)}"` : ""}>${osIcon(j.os)}</span>
        <span class="name">${esc(j.name)}</span>
        <span class="host">${esc(j.host)}</span></div>`;
          })
          .join("")
      : `<p class="empty">no match</p>`;
  presultsEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}
