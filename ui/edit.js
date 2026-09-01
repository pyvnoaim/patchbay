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
    select(+jackRow.dataset.i);
    const j = shown[sel];
    return showCtx(e.clientX, e.clientY, j.name, [
      { icon: "square-terminal", label: "Connect", run: () => connect(j.name) },
      { icon: "external-link", label: "Open in Terminal", run: () => connect(j.name, true) },
      ...(j.url ? [{ icon: "globe", label: "Open web UI", run: () => openWeb(j.name) }] : []),
      ...(j.rdp ? [
        { icon: "monitor", label: "Remote desktop", run: () => openRdp(j.name) },
        { icon: "external-link", label: "Remote desktop in system client", run: () => handOffRdp(j.name) },
      ] : []),
      { icon: "copy", label: "Copy ssh command", run: () => navigator.clipboard.writeText(j.command).catch(() => {}) },
      "-",
      { icon: "plug", label: "Ping", run: () => openSession(j.name, "ping") },
      { icon: "waypoints", label: "Trace route", run: () => openSession(j.name, "trace") },
      "-",
      { icon: "pencil", label: "Edit…", run: () => openJack(j) },
      { icon: "trash-2", label: "Delete", danger: true, run: () => removeJack(j.name) },
    ]);
  }

  if (groupRow) {
    const path = groupRow.dataset.path;
    if (!path) return;   // "All jacks" isn't a real folder
    return showCtx(e.clientX, e.clientY, path, [
      { icon: "plus", label: "New device here…", run: () => openJack(null, path) },
      { icon: "folder-plus", label: "New subfolder…", run: () => newGroup(path) },
      "-",
      { icon: "pencil", label: "Rename…", run: () => renameGroup(path) },
      { icon: "plug", label: vpns.has(path) ? "VPN settings…" : "Add a VPN…", run: () => openVpn(path) },
      { icon: "trash-2", label: "Delete folder", danger: true, run: () => removeGroup(path) },
    ]);
  }

  showCtx(e.clientX, e.clientY, null, [
    { icon: "plus", label: "New device…", run: () => openJack(null, group) },
    { icon: "folder-plus", label: "New folder…", run: () => newGroup(null) },
    "-",
    { icon: "download", label: "Import from ssh config…", run: () => openImport() },
    { icon: "file-pen-line", label: "Open config file", run: () => invoke("open_config") },
  ]);
});
window.addEventListener("blur", hideCtx);
document.addEventListener("mousedown", (e) => { if (!e.target.closest("#ctx")) hideCtx(); });
window.addEventListener("resize", hideCtx);

// ── ask (one-line prompt) ──────────────────────────────────────────────────
let askResolve = null;
// A prompt when there is something to type, a plain confirmation when `value` is
// null. Pre-filling a box with the answer and then checking you typed it back is
// ceremony, not a safeguard — the button label already says what will happen.
/// A `user` of null is the plain one-input prompt; a string (empty included) adds
/// the username field above and resolves to `{ user, password }` instead.
function ask(title, value = "", okLabel = "OK", type = "text", user = null) {
  const confirming = value === null;
  $("ask-title").textContent = title;
  askBody.hidden = confirming;
  askInput.type = type;
  askInput.value = confirming ? "" : value;
  askUserField.hidden = user === null;
  askUser.value = user ?? "";
  askLabel.hidden = user === null;
  askLabel.textContent = "Password";
  askErr.hidden = true;
  $("ask-ok").textContent = okLabel;
  askWrap.hidden = false;
  if (confirming) $("ask-ok").focus();
  else if (user === "") askUser.focus();
  else { askInput.focus(); askInput.select(); }
  return new Promise((res) => (askResolve = res));
}
function closeAsk(v) { askWrap.hidden = true; askResolve?.(v); askResolve = null; }
askForm.addEventListener("submit", (e) => {
  e.preventDefault();
  if (askBody.hidden) return closeAsk(true);
  if (askUserField.hidden) return closeAsk(askInput.value.trim() || null);
  const user = askUser.value.trim();
  if (!user) return showErr(askErr, "a username, or the desktop won't let you in");
  // The password is the one field that isn't trimmed — a space in one is a character.
  closeAsk(askInput.value ? { user, password: askInput.value } : null);
});
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
  // Editing reflects what the device already has; a new one starts at terminal.
  f.use_ssh.checked = j ? j.ssh : true;
  f.use_rdp.checked = !!j?.rdp;
  f.use_web.checked = !!j?.url;
  f.name.value = j?.name ?? "";
  f.host.value = j?.host ?? "";
  f.user.value = j?.user ?? "";
  f.port.value = j?.port ?? "";
  f.key.value = j?.key ?? "";
  f.jump.value = j?.jump ?? "";
  f.os.value = j?.os ?? "";
  // The scheme is a control, not something to type — and not something to typo.
  const m = /^(https?:\/\/)(.*)$/i.exec(j?.url ?? "");
  setScheme(m ? m[1].toLowerCase() : "https://");
  f.url.value = m ? m[2] : "";
  f.rdp.value = j?.rdp ?? "";
  f.primary.value = j?.primary ?? "ssh";
  f.desc.value = j?.desc ?? "";
  f.folders.value = (j?.folders ?? (prefillGroup ? [prefillGroup] : [])).join(", ");
  f.forward.value = (j?.forward ?? []).join(", ");
  $("oschoices").innerHTML = OS_CHOICES.map((o) => `<option value="${esc(o)}">`).join("");
  renderFolderSuggestions();
  jackFields();
  sheetWrap.hidden = false;
  f.name.focus();
}
const closeJack = () => { sheetWrap.hidden = true; };
const list2 = (s) => s.split(",").map((x) => x.trim()).filter(Boolean);

jackForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = jackForm.elements;
  const on = { ssh: f.use_ssh.checked, rdp: f.use_rdp.checked, web: f.use_web.checked };
  if (!on.ssh && !on.rdp && !on.web) return showErr(jfErr, "pick at least one way to reach it");
  const port = f.port.value.trim();
  const rdp = on.rdp ? f.rdp.value.trim() : "";
  if (on.ssh && port && !/^\d+$/.test(port)) return showErr(jfErr, "ssh port has to be a number");
  if (on.rdp && !/^\d+$/.test(rdp)) return showErr(jfErr, "rdp port has to be a number");
  try {
    await invoke("save_jack", {
      original: editing,
      jack: {
        name: f.name.value.trim(),
        host: f.host.value.trim(),
        user: f.user.value.trim() || null,
        // A protocol that is off must clear its fields, not leave them lying about.
        port: on.ssh && port ? +port : null,
        key: on.ssh ? f.key.value.trim() || null : null,
        jump: on.ssh ? f.jump.value.trim() || null : null,
        forward: on.ssh ? list2(f.forward.value) : [],
        os: f.os.value.trim() || null,
        url: on.web && f.url.value.trim() ? scheme + f.url.value.trim() : null,
        rdp: rdp ? +rdp : null,
        ssh: on.ssh ? null : false,
        // Only stored when it differs from "the first one it has".
        primary: (() => {
          const first = ["ssh", "rdp", "web"].find((k) => on[k]);
          return f.primary.value !== first ? f.primary.value : null;
        })(),
        desc: f.desc.value.trim() || null,
        folders: list2(f.folders.value),
      },
    });
    for (const f2 of list2(f.folders.value)) pending.delete(f2);
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
  if (!(await ask(`Delete "${name}"? This edits your config file.`, null, "Delete"))) return;
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
  const to = await ask(`Rename folder ${path} to`, path.split("/").pop(), "Rename");
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
  const n = all.filter((j) => j.folders.some((f) => f === path || f.startsWith(path + "/"))).length;
  const msg = `Remove folder "${path}" from ${n} device${n === 1 ? "" : "s"}? The devices stay.`;
  if (!(await ask(msg, null, "Remove"))) return;
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

// ── import sheet ───────────────────────────────────────────────────────────
// The CLI prints TOML and you paste it; the window has somewhere to show the list,
// so it ticks and writes instead — through save_jack, like every other edit.
let impFound = [];

async function openImport() {
  impFound = [];
  impErr.hidden = true;
  impList.innerHTML = "";
  impNote.textContent = "Reading your ssh config…";
  impOk.disabled = true;
  impWrap.hidden = false;

  let r;
  try {
    r = await invoke("ssh_hosts");
  } catch (err) {
    impNote.textContent = "";
    return showErr(impErr, String(err));
  }

  // A host already in the config is shown but not ticked, so running this twice is
  // safe and you can see what it would have added.
  impFound = r.hosts.map((h) => ({ ...h, here: all.some((j) => j.name === h.name) }));
  const fresh = impFound.filter((h) => !h.here).length;
  impNote.innerHTML =
    `<b>${impFound.length}</b> host${impFound.length === 1 ? "" : "s"} in ${esc(r.path)}` +
    `${fresh < impFound.length ? ` · ${impFound.length - fresh} already here` : ""}` +
    r.warnings.map((w) => `<span class="warn">${esc(w)}</span>`).join("");

  impList.innerHTML = impFound.map((h, i) => `
    <label class="imp-row">
      <input type="checkbox" data-i="${i}"${h.here ? " disabled" : " checked"}>
      <span class="imp-name">${esc(h.name)}</span>
      <span class="imp-host">${esc(h.user ? `${h.user}@${h.host}` : h.host)}${h.port ? `:${h.port}` : ""}</span>
      ${h.here ? `<span class="imp-tag">already here</span>` : ""}
    </label>`).join("");
  impOk.disabled = !fresh;
}

const closeImport = () => { impWrap.hidden = true; };
$("imp-cancel").addEventListener("click", closeImport);
impWrap.addEventListener("mousedown", (e) => { if (e.target === impWrap) closeImport(); });
$("imp-all").addEventListener("click", () => {
  const boxes = [...impList.querySelectorAll("input:not(:disabled)")];
  const to = !boxes.every((b) => b.checked);
  for (const b of boxes) b.checked = to;
});

impForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const picked = [...impList.querySelectorAll("input:checked")].map((b) => impFound[+b.dataset.i]);
  if (!picked.length) return showErr(impErr, "nothing ticked to import");

  impOk.disabled = true;
  impOk.textContent = "Importing…";
  // ponytail: one write per host, because save_jack is the only writer and a failure
  // then names the host it was on. A bulk writer when someone imports enough for the
  // rewrites to show.
  const failed = [];
  for (const h of picked) {
    try {
      await invoke("save_jack", {
        original: null,
        jack: {
          name: h.name, host: h.host,
          user: h.user ?? null, port: h.port ?? null,
          key: h.key ?? null, jump: h.jump ?? null,
          folders: [], forward: [],
        },
      });
    } catch { failed.push(h.name); }
  }
  impOk.textContent = "Import";
  await load();

  if (!failed.length) return closeImport();
  impOk.disabled = false;
  showErr(impErr, `${picked.length - failed.length} imported, ${failed.length} refused: ${failed.join(", ")}`);
});

// ── vpn sheet ──────────────────────────────────────────────────────────────
let vpnEditing = null;

/// Show only the fields the chosen protocols actually need.
function jackFields() {
  const f = jackForm.elements;
  const on = { ssh: f.use_ssh.checked, rdp: f.use_rdp.checked, web: f.use_web.checked };
  for (const el of jackForm.querySelectorAll("[data-need]")) {
    el.hidden = !el.dataset.need.split(" ").some((k) => on[k]);
  }
  // Sensible starting point rather than an empty box you have to know to fill.
  if (on.rdp && !f.rdp.value.trim()) f.rdp.value = "3389";

  // Only worth asking when there is actually a choice to make.
  const opts = [["ssh", "Terminal"], ["rdp", "Remote desktop"], ["web", "Web UI"]].filter(([k]) => on[k]);
  $("primary-row").hidden = opts.length < 2;
  const keep = f.primary.value;
  f.primary.innerHTML = opts.map(([k, l]) => `<option value="${k}">${l}</option>`).join("");
  f.primary.value = opts.some(([k]) => k === keep) ? keep : opts[0]?.[0] ?? "ssh";
}

function vpnFields() {
  const f = vpnForm.elements;
  const p = providers.find((x) => x.id === f.provider.value);
  const custom = !p || p.id === "custom";
  // Presets derive the commands, so only Custom shows the raw three.
  for (const name of ["up", "down", "check"]) f[name].closest(".f").hidden = !custom;
  $("vpn-profile-row").hidden = custom || !p.needs_profile;
  $("vpn-profiles").innerHTML = (p?.profiles ?? []).map((n) => `<option value="${esc(n)}">`).join("");
}

async function openVpn(path) {
  vpnEditing = path;
  $("vpn-title").textContent = `VPN for ${path}`;
  vpnErr.hidden = true;
  vpnDelete.hidden = !vpns.has(path);
  vpnDelete.innerHTML = `${icon("trash-2")}Remove`;
  const f = vpnForm.elements;
  f.provider.innerHTML = providers
    .map((p) => `<option value="${esc(p.id)}"${p.installed ? "" : " disabled"}>${esc(p.label)}${p.installed ? "" : " — not installed"}</option>`)
    .join("");
  f.provider.value = "custom";
  f.profile.value = f.up.value = f.down.value = f.check.value = "";
  vpnFields();
  vpnWrap.hidden = false;
  try {
    const def = await invoke("vpn_def", { path });
    if (def && vpnEditing === path) {
      f.provider.value = def.provider ?? "custom";
      f.profile.value = def.profile ?? "";
      f.up.value = def.up ?? "";
      f.down.value = def.down ?? "";
      f.check.value = def.check ?? "";
      vpnFields();
    }
  } catch (e) { showErr(vpnErr, String(e)); }
}
vpnForm.elements.provider.addEventListener("change", vpnFields);
const closeVpn = () => { vpnWrap.hidden = true; };

vpnForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = vpnForm.elements;
  try {
    await invoke("save_vpn", {
      path: vpnEditing,
      def: {
        provider: f.provider.value,
        profile: f.profile.value.trim() || null,
        up: f.up.value.trim() || null,
        down: f.down.value.trim() || null,
        check: f.check.value.trim() || null,
      },
    });
    closeVpn();
    await refreshVpns();
    syncTeam();
  } catch (err) { showErr(vpnErr, String(err)); }
});
$("vpn-cancel").addEventListener("click", closeVpn);
vpnWrap.addEventListener("mousedown", (e) => { if (e.target === vpnWrap) closeVpn(); });
vpnDelete.addEventListener("click", async () => {
  const path = vpnEditing;
  if (!(await ask(`Remove the VPN on "${path}"? The folder and its devices stay.`, null, "Remove"))) return;
  closeVpn();
  try { await invoke("delete_vpn", { path }); await refreshVpns(); syncTeam(); }
  catch (e) { alertish(e); }
});

// ── settings ───────────────────────────────────────────────────────────────
async function openSettings() {
  setErr.hidden = true;
  for (const [k, v] of Object.entries(prefs)) {
    if (setForm.elements[k]) setForm.elements[k].checked = !!v;
  }
  // Show what this machine can actually drive, so "why is Tunnelblick greyed out?"
  // has an answer without leaving the sheet.
  $("provider-list").innerHTML = providers
    .filter((p) => p.id !== "custom")
    .map((p) => `<span class="prov ${p.installed ? "on" : ""}">${icon(p.installed ? "check" : "x")}${esc(p.label)}</span>`)
    .join("");
  // Read fresh rather than from state: nothing else in the app needs [defaults],
  // and a hand-edit between openings should show up here.
  const defs = await invoke("defaults").catch(() => ({}));
  for (const k of DEFAULT_KEYS) setForm.elements[`def_${k}`].value = defs[k] ?? "";
  renderSwatches();
  teamErr.hidden = true;
  renderTeam();
  // Land on the thing that needs answering. The sidebar button was already warning
  // about it, so opening on VPN would make you hunt for what you clicked it for.
  showPane(TEAM_STUCK[team.state] ? "team" : "vpn");
  syncTeam();   // seats and state, fresh, while the sheet is already up
  $("page-openconfig").innerHTML = `${icon("file-pen-line")}Open config file`;
  setWrap.hidden = false;
  try { $("cfgpath").textContent = await invoke("config_path"); } catch { /* shown blank */ }
}
const closeSettings = () => { setWrap.hidden = true; };

// Six unrelated sections were one scroll. A hidden pane is still in the form, so
// `setForm.elements` sees every field either way and Save stays one submit.
function showPane(name) {
  for (const b of setNav.querySelectorAll("button")) {
    // The label is the button's own text; an <svg> holds none, so this stays put
    // on the second call rather than nesting an icon inside an icon.
    if (!b.firstElementChild) b.innerHTML = `${icon(b.dataset.icon)}${esc(b.textContent)}`;
    b.classList.toggle("on", b.dataset.pane === name);
  }
  for (const p of setForm.querySelectorAll(".pane")) p.hidden = p.dataset.pane !== name;
}
setNav.addEventListener("click", (e) => {
  const b = e.target.closest("button[data-pane]");
  if (b) showPane(b.dataset.pane);
});

const DEFAULT_KEYS = ["user", "port", "key", "jump"];

setForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const next = {};
  for (const el of setForm.querySelectorAll("input[type=checkbox]")) next[el.name] = el.checked;

  const defs = {};
  for (const k of DEFAULT_KEYS) defs[k] = setForm.elements[`def_${k}`].value.trim() || null;
  if (defs.port && !/^\d+$/.test(defs.port)) return showErr(setErr, "ssh port has to be a number");
  defs.port = defs.port ? +defs.port : null;

  try {
    const wasProbing = prefs.probe !== false;
    await invoke("save_settings", { next });
    await invoke("save_defaults", { next: defs });
    prefs = next;
    closeSettings();
    // A reload, not a render: [defaults] merges into every jack, so changing it
    // changes the user, port and route shown for all of them.
    await load();
    // The throttle would otherwise hold the first sweep back by up to 30s.
    if (!wasProbing && next.probe) { lastProbe = 0; refreshProbes(); }
  } catch (err) { showErr(setErr, String(err)); }
});
$("set-cancel").addEventListener("click", closeSettings);
$("page-openconfig").addEventListener("click", () => invoke("open_config"));
setWrap.addEventListener("mousedown", (e) => { if (e.target === setWrap) closeSettings(); });

// ── team ───────────────────────────────────────────────────────────────────
// The config file is the shared document. Everything here is one call away from
// team_sync, which is the only thing in the app that talks to the server.
function renderTeam() {
  // Same warning as the sidebar button, on the rail that now stands between them.
  setNav.querySelector('[data-pane="team"]').classList.toggle("warn", !!TEAM_STUCK[team.state]);
  const joined = team.state !== "off";
  $("team-off").hidden = joined;
  $("team-on").hidden = !joined;
  if (!joined) return;
  $("team-leave").innerHTML = `${icon("unplug")}Leave the team`;
  $("team-where").textContent = `${team.code}   ${team.url}`;
  $("team-fix").hidden = team.state !== "conflict";
  const seats = `${team.seats} seat${team.seats === 1 ? "" : "s"}${team.paid ? "" : ", free up to three"}`;
  const say = {
    conflict: "Your list and the team's have both changed since they last agreed. Pick one — " +
      "whichever you drop is kept beside your config as patchbay.toml.bak.",
    blocked: `${team.error ?? ""} Your edits stay on this machine until the team has room for them.`,
    offline: `Not reaching the server: ${team.error ?? ""} — your list still works, and changes go up when it answers.`,
    // Nothing to do with the server, so don't blame it: this machine's own config is
    // in the way, and nothing syncs either direction until it's readable again.
    error: `${team.error ?? "the sync stopped here"} — nothing is going up or coming down until that's sorted.`,
  };
  $("team-state").textContent = say[team.state] ?? `In sync · ${seats}`;
}

async function teamCall(fn) {
  teamErr.hidden = true;
  try {
    team = await fn();
    renderTeam();
    await load();
  } catch (e) { showErr(teamErr, String(e)); }
}

const teamUrl = () => $("team-url").value.trim();
$("team-join").addEventListener("click", () =>
  teamCall(() => invoke("team_join", { url: teamUrl(), code: $("team-code").value.trim() })));
$("team-create").addEventListener("click", () =>
  teamCall(() => invoke("team_create", { url: teamUrl() })));
$("team-theirs").addEventListener("click", () =>
  teamCall(() => invoke("team_resolve", { keep: "theirs" })));
$("team-mine").addEventListener("click", () =>
  teamCall(() => invoke("team_resolve", { keep: "mine" })));
$("team-leave").addEventListener("click", async () => {
  if (!(await ask("Leave the team? Your copy of the list stays on this machine.", null, "Leave"))) return;
  teamCall(async () => { await invoke("team_leave"); return { state: "off" }; });
});

// Every OS actually in use, plus anything already overridden.
function renderSwatches() {
  const inUse = [...new Set(all.map((j) => osKey(j.os)).filter(Boolean))];
  const keys = [...new Set([...inUse, ...Object.keys(colors)])].sort();
  $("swatches").innerHTML = keys.length
    ? keys.map((k) => {
        const shown = osColor(k) ?? "#8b8b95";
        const overridden = k in colors;
        return `<span class="sw" data-os="${esc(k)}">
          <input type="color" value="${esc(/^#[0-9a-f]{6}$/i.test(shown) ? shown : "#8b8b95")}">
          <span class="mark" style="color:${esc(shown)}">${osIcon(k)}</span>${esc(k)}
          ${overridden ? `<i class="reset" data-reset="${esc(k)}" data-tip="Back to the brand colour">${icon("x")}</i>` : ""}
        </span>`;
      }).join("")
    : `<p class="page-note">No devices have an <code>os</code> set yet.</p>`;
}

$("swatches").addEventListener("input", async (e) => {
  const sw = e.target.closest("[data-os]");
  if (!sw || e.target.type !== "color") return;
  await setColor(sw.dataset.os, e.target.value);
});
$("swatches").addEventListener("click", async (e) => {
  const key = e.target.closest("[data-reset]")?.dataset.reset;
  if (key) await setColor(key, null);
});

async function setColor(os, hex) {
  try {
    await invoke("save_color", { os, hex });
    colors = await invoke("colors");
    renderSwatches();
    render();
    syncTeam();   // [colors] is the team's, and nothing here goes through load()
  } catch (err) { showErr(setErr, String(err)); }
}

for (const n of ["use_ssh", "use_rdp", "use_web"]) {
  jackForm.elements[n].addEventListener("change", jackFields);
}

// ── url scheme + folders ───────────────────────────────────────────────────
let scheme = "https://";
function setScheme(next) {
  scheme = next;
  const btn = $("url-scheme");
  btn.textContent = scheme;
  btn.dataset.tip = `Switch to ${scheme === "https://" ? "http://" : "https://"}`;
}
$("url-scheme").addEventListener("click", () =>
  setScheme(scheme === "https://" ? "http://" : "https://"));

// Suggestions are a plain datalist, same as the OS field. What you have entered
// is shown underneath as paths, so nesting is legible without a popup panel.
function renderFolderSuggestions() {
  const used = [...new Set(all.flatMap((j) => j.folders))].sort();
  $("folderlist").innerHTML = used.map((f) => `<option value="${esc(f)}">`).join("");
  renderCrumbs();
}

const enteredFolders = () =>
  jackForm.elements.folders.value.split(",").map((x) => x.trim()).filter(Boolean);

function renderCrumbs() {
  $("crumbs").innerHTML = enteredFolders().map((f) => {
    const parts = f.split("/").filter(Boolean);
    const path = parts
      .map((p, i) => `<span class="${i === parts.length - 1 ? "leafname" : ""}">${esc(p)}</span>`)
      .join(`<span class="sep">›</span>`);
    return `<span class="crumb">${path}<i class="drop" data-drop="${esc(f)}"
      data-tip="Remove">${icon("x")}</i></span>`;
  }).join("");
}

jackForm.elements.folders.addEventListener("input", renderCrumbs);
$("crumbs").addEventListener("click", (e) => {
  const dropped = e.target.closest("[data-drop]")?.dataset.drop;
  if (!dropped) return;
  jackForm.elements.folders.value = enteredFolders().filter((f) => f !== dropped).join(", ");
  renderCrumbs();
});
