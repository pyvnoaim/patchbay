// Classic script, no bundler — see the load order in ui/index.html.
// Everything that changes the config: right-click menu, prompts, the
// add/edit sheet, and the calls into config.rs.

// ── context menu ───────────────────────────────────────────────────────────
const actions = new Map();   // id -> fn, rebuilt each time the menu opens

function showCtx(x, y, head, items) {
  actions.clear();
  ctxEl.innerHTML =
    (head ? `<div class="ctx-head">${esc(head)}</div><div class="ctx-sep"></div>` : "") +
    items.map((it, i) => {
      if (it === "-") return `<div class="ctx-sep"></div>`;
      actions.set(String(i), it.run);
      return `<div class="ctx-item ${it.danger ? "danger" : ""}" data-a="${i}">${icon(it.icon)}${esc(it.label)}</div>`;
    }).join("");
  ctxEl.hidden = false;
  // Flip back inside the window rather than overflowing it.
  const r = ctxEl.getBoundingClientRect();
  ctxEl.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  ctxEl.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
}
const hideCtx = () => { ctxEl.hidden = true; };

ctxEl.addEventListener("click", (e) => {
  const a = e.target.closest("[data-a]")?.dataset.a;
  hideCtx();
  actions.get(a)?.();
});

// The webview's own menu is Reload / Inspect Element — never useful here.
document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  const jackRow = e.target.closest("#list .jack");
  const groupRow = e.target.closest(".group");

  if (jackRow) {
    sel = +jackRow.dataset.i;
    render();
    const j = shown[sel];
    return showCtx(e.clientX, e.clientY, j.name, [
      { icon: "square-terminal", label: "Connect", run: () => connect(j.name) },
      { icon: "external-link", label: "Open in Terminal", run: () => connect(j.name, true) },
      { icon: "copy", label: "Copy ssh command", run: () => navigator.clipboard.writeText(j.command).catch(() => {}) },
      "-",
      { icon: "pencil", label: "Edit…", run: () => openJack(j) },
      { icon: "trash-2", label: "Delete", danger: true, run: () => removeJack(j.name) },
    ]);
  }

  if (groupRow) {
    const path = groupRow.dataset.path;
    if (!path || path === "\0untagged") return;   // All jacks / Untagged aren't real folders
    return showCtx(e.clientX, e.clientY, path, [
      { icon: "plus", label: "New device here…", run: () => openJack(null, path) },
      { icon: "folder-plus", label: "New subfolder…", run: () => newGroup(path) },
      "-",
      { icon: "pencil", label: "Rename…", run: () => renameGroup(path) },
      { icon: "trash-2", label: "Delete folder", danger: true, run: () => removeGroup(path) },
    ]);
  }

  showCtx(e.clientX, e.clientY, null, [
    { icon: "plus", label: "New device…", run: () => openJack(null, group) },
    { icon: "folder-plus", label: "New folder…", run: () => newGroup(null) },
    "-",
    { icon: "file-pen-line", label: "Open config file", run: () => invoke("open_config") },
  ]);
});
window.addEventListener("blur", hideCtx);
document.addEventListener("mousedown", (e) => { if (!e.target.closest("#ctx")) hideCtx(); });
window.addEventListener("resize", hideCtx);

// ── ask (one-line prompt) ──────────────────────────────────────────────────
let askResolve = null;
function ask(title, value = "", okLabel = "OK") {
  $("ask-title").textContent = title;
  askInput.value = value;
  askErr.hidden = true;
  $("ask-ok").textContent = okLabel;
  askWrap.hidden = false;
  askInput.focus();
  askInput.select();
  return new Promise((res) => (askResolve = res));
}
function closeAsk(v) { askWrap.hidden = true; askResolve?.(v); askResolve = null; }
askForm.addEventListener("submit", (e) => { e.preventDefault(); closeAsk(askInput.value.trim() || null); });
$("ask-cancel").addEventListener("click", () => closeAsk(null));
askWrap.addEventListener("mousedown", (e) => { if (e.target === askWrap) closeAsk(null); });

// ── jack sheet ─────────────────────────────────────────────────────────────
function openJack(j, prefillGroup) {
  editing = j?.name ?? null;
  $("sheet-title").textContent = j ? `Edit ${j.name}` : "New device";
  jfDelete.hidden = !j;
  jfDelete.innerHTML = `${icon("trash-2")}Delete`;
  jfErr.hidden = true;
  const f = jackForm.elements;
  f.name.value = j?.name ?? "";
  f.host.value = j?.host ?? "";
  f.user.value = j?.user ?? "";
  f.port.value = j?.port ?? "";
  f.key.value = j?.key ?? "";
  f.jump.value = j?.jump ?? "";
  f.os.value = j?.os ?? "";
  f.desc.value = j?.desc ?? "";
  f.tags.value = (j?.tags ?? (prefillGroup && prefillGroup !== "\0untagged" ? [prefillGroup] : [])).join(", ");
  f.forward.value = (j?.forward ?? []).join(", ");
  $("jacknames").innerHTML = all.map((x) => `<option value="${esc(x.name)}">`).join("");
  $("oschoices").innerHTML = OS_CHOICES.map((o) => `<option value="${o}">`).join("");
  sheetWrap.hidden = false;
  f.name.focus();
}
const closeJack = () => { sheetWrap.hidden = true; };
const list2 = (s) => s.split(",").map((x) => x.trim()).filter(Boolean);

jackForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = jackForm.elements;
  const port = f.port.value.trim();
  if (port && !/^\d+$/.test(port)) return showErr(jfErr, "port has to be a number");
  try {
    await invoke("save_jack", {
      original: editing,
      jack: {
        name: f.name.value.trim(),
        host: f.host.value.trim(),
        user: f.user.value.trim() || null,
        port: port ? +port : null,
        key: f.key.value.trim() || null,
        jump: f.jump.value.trim() || null,
        os: f.os.value.trim() || null,
        desc: f.desc.value.trim() || null,
        tags: list2(f.tags.value),
        forward: list2(f.forward.value),
      },
    });
    for (const t of list2(f.tags.value)) pending.delete(t);
    closeJack();
    await load();
  } catch (err) { showErr(jfErr, String(err)); }
});
$("jf-cancel").addEventListener("click", closeJack);
sheetWrap.addEventListener("mousedown", (e) => { if (e.target === sheetWrap) closeJack(); });
jfDelete.addEventListener("click", () => { const n = editing; closeJack(); removeJack(n); });

function showErr(el, msg) { el.textContent = msg; el.hidden = false; }

// ── mutations ──────────────────────────────────────────────────────────────
async function removeJack(name) {
  if ((await ask(`Delete "${name}"? This edits your config file.`, name, "Delete")) !== name) return;
  try { await invoke("delete_jack", { name }); sel = 0; await load(); }
  catch (e) { alertish(e); }
}

async function newGroup(parent) {
  const name = await ask(parent ? `New folder inside ${parent}` : "New folder", "", "Create");
  if (!name) return;
  const path = parent ? `${parent}/${name.replace(/^\/+|\/+$/g, "")}` : name.replace(/^\/+|\/+$/g, "");
  pending.add(path);
  expanded.add(path.split("/")[0]);
  group = path;
  render();
}

async function renameGroup(path) {
  const to = await ask(`Rename ${path} to`, path.split("/").pop(), "Rename");
  if (!to) return;
  const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  const next = parent ? `${parent}/${to}` : to;
  try {
    await invoke("rename_group", { from: path, to: next });
    if (pending.delete(path)) pending.add(next);
    if (group === path) group = next;
    await load();
  } catch (e) { alertish(e); }
}

async function removeGroup(path) {
  const n = all.filter((j) => j.tags.some((t) => t === path || t.startsWith(path + "/"))).length;
  const msg = `Remove folder "${path}" from ${n} device${n === 1 ? "" : "s"}? The devices stay.`;
  if ((await ask(msg, path, "Remove")) !== path) return;
  try {
    await invoke("delete_group", { path });
    pending.delete(path);
    if (group === path || group?.startsWith(path + "/")) group = null;
    await load();
  } catch (e) { alertish(e); }
}

function alertish(e) {
  const box = detailEl.querySelector(".mono");
  if (box) { box.textContent = String(e); box.classList.add("err"); }
}
