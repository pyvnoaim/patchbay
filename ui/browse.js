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

function renderTree() {
  const rows = [`<div class="side-title">Patchbay</div>`,
    row({ name: "All jacks", path: null, members: new Set(all.map((j) => j.name)), children: new Map() }, 0, "layers")];

  const walk = (level, depth) => {
    for (const node of [...level.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      rows.push(row(node, depth));
      if (expanded.has(node.path)) walk(node.children, depth + 1);
    }
  };
  const tree = buildTree();
  if (tree.size) rows.push(`<div class="side-title">Groups</div>`);
  walk(tree, 0);

  const untagged = all.filter((j) => !j.tags.length).length;
  if (untagged) rows.push(row({ name: "Untagged", path: "\0untagged", members: new Set(Array(untagged)), children: new Map() }, 0, "circle-off"));
  treeEl.innerHTML = rows.join("");
}

function row(node, depth, glyph) {
  const kids = node.children.size > 0;
  const open = expanded.has(node.path);
  const g = glyph ?? (kids ? (open ? "folder-open" : "folder") : "tag");
  return `<div class="group" data-path="${esc(node.path ?? "")}" data-has-kids="${kids}"
       aria-current="${group === node.path}" style="padding-left:${8 + depth * 13}px">
    <span class="twist ${kids ? "" : "leaf"} ${open ? "open" : ""}">${icon("chevron-right")}</span>
    <span class="gi">${icon(g)}</span>
    <span class="label">${esc(node.name)}</span>
    <span class="n">${node.members.size}</span>
  </div>`;
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
  $("newgroup").title = "New folder";
  $("editcfg").innerHTML = icon("file-pen-line");
  $("editcfg").title = `Open the config file (${chord("e")})`;

  if (!shown.length) {
    listEl.innerHTML = `<p class="empty">${all.length ? "nothing here" : "no jacks yet — <code>bay edit</code>"}</p>`;
    renderDetail();   // a session tab still has something to describe
    return;
  }
  sel = Math.min(sel, shown.length - 1);
  listEl.innerHTML = shown.map((j, i) => {
    const p = probes.get(j.name);
    const state = !p ? "unknown" : p.ms == null ? "down" : "up";
    return `<div class="jack" data-i="${i}" aria-selected="${i === sel}">
      <span class="dot ${state}"></span>
      <span class="os" title="${esc(j.os ?? "")}">${osIcon(j.os)}</span>
      <span class="name">${esc(j.name)}</span>
      <span class="host">${esc(j.user ? j.user + "@" + j.host : j.host)}${j.port ? ":" + j.port : ""}</span>
      <span class="tags">${j.tags.map((t) => `<span class="tag">${esc(t.split("/").pop())}</span>`).join("")}</span>
    </div>`;
  }).join("");
  renderDetail();
  listEl.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

function renderDetail() {
  // On a session tab the pane describes that session's jack, not the list selection.
  const live = activeId !== null ? sessions.get(activeId) : null;
  const j = live ? all.find((x) => x.name === live.name) : shown[sel];
  if (!j) return (detailEl.innerHTML = "");
  const p = probes.get(j.name);
  const reach = !p ? `<span style="color:var(--fg-faint)">checking…</span>`
    : p.ms == null ? `<span style="color:var(--down)">no answer</span> · ${esc(p.target)}`
    : `<span style="color:var(--up)">up</span> · ${esc(p.target)} · ${p.ms}ms`;

  const stops = [...j.hops, j.user ? `${j.user}@${j.host}` : j.host];
  detailEl.innerHTML = `
    <div class="d-name"><span class="d-os">${osIcon(j.os)}</span>${esc(j.name)}</div>
    ${j.desc ? `<div class="d-desc">${esc(j.desc)}</div>` : `<div class="d-desc"></div>`}

    <div class="d-sec">${icon("server")}Target</div>
    <dl>
      <div class="d-row"><dt>host</dt><dd>${esc(j.host)}</dd></div>
      ${j.user ? `<div class="d-row"><dt>user</dt><dd>${esc(j.user)}</dd></div>` : ""}
      ${j.port ? `<div class="d-row"><dt>port</dt><dd>${j.port}</dd></div>` : ""}
      ${j.key ? `<div class="d-row"><dt>key</dt><dd>${esc(j.key)}</dd></div>` : ""}
    </dl>

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

    <div class="d-sec">${icon("square-terminal")}Command</div>
    <div class="mono ${j.command.startsWith("ssh ") ? "" : "err"}">${esc(j.command)}</div>
    <div class="btns">
      ${live
        ? `<button class="primary" data-act="disconnect">${icon("x")}${live.dead ? "Close tab" : "Disconnect"}</button>`
        : `<button class="primary" data-act="connect">${icon("square-terminal")}Connect</button>`}
      <button class="ghost" data-act="edit" title="Edit">${icon("pencil")}</button>
      <button class="ghost" data-act="copy" title="Copy command">${icon("copy")}</button>
    </div>`;
}

function move(d) {
  if (!shown.length) return;
  sel = (sel + d + shown.length) % shown.length;
  render();
}

// In-app by default; `inTerminal` hands off to Terminal.app / wt / gnome-terminal.
async function connect(name, inTerminal = false) {
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
