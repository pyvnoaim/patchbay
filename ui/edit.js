// Everything that changes the config: context menu, prompts, the device sheet, settings
// and import. Classic script; see the load order in ui/index.html.

// ── context menu ───────────────────────────────────────────────────────────
const actions = new Map(); // id -> fn, rebuilt each time the menu opens

// `os` is passed only for a device row, so the menu wears the same mark as the list.
function showCtx(x, y, head, items, os) {
  actions.clear();
  const mark =
    os === undefined
      ? ""
      : `<span class="ctx-os"${osColor(os) ? ` style="color:${esc(osColor(os))}"` : ""}>${osIcon(os)}</span>`;
  ctxEl.innerHTML =
    (head
      ? `<div class="ctx-head">${mark}<span class="ctx-name">${esc(head)}</span></div><div class="ctx-sep"></div>`
      : "") +
    items
      .map((it, i) => {
        if (it === "-") return `<div class="ctx-sep"></div>`;
        actions.set(String(i), it.run);
        const key = it.key ? `<kbd>${esc(it.key)}</kbd>` : "";
        return `<div class="ctx-item ${it.danger ? "danger" : ""}" data-a="${i}">${icon(it.icon)}${esc(it.label)}${key}</div>`;
      })
      .join("");
  ctxEl.hidden = false;
  // Flip back inside the window rather than overflowing it.
  const r = ctxEl.getBoundingClientRect();
  ctxEl.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  ctxEl.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
}
const hideCtx = () => {
  ctxEl.hidden = true;
};

// The pointer takes over from the keyboard, or two rows are highlighted at once.
ctxEl.addEventListener("mouseover", () => {
  ctxEl.querySelector(".ctx-item.on")?.classList.remove("on");
});

ctxEl.addEventListener("click", (e) => {
  const a = e.target.closest("[data-a]")?.dataset.a;
  hideCtx();
  actions.get(a)?.();
});

// The webview's own menu is Reload / Inspect Element.
document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  const jackRow = e.target.closest("#list .jack");
  const groupRow = e.target.closest(".group");

  if (jackRow && jackRow.dataset.i !== undefined) {
    const at = +jackRow.dataset.i;
    // Right-clicking an unmarked row is an ordinary click: it selects and drops the marks.
    if (!marked.has(shown[at]?.name)) marked.clear();
    select(at);
    const bulk = markedHere();
    if (bulk.length > 1) {
      const ssh = bulk.filter((x) => x.ssh);
      return showCtx(e.clientX, e.clientY, `${bulk.length} devices`, [
        // Broadcast needs two ssh marks. Non-ssh marks are named in a pill and skipped: a
        // webview is an OS view above the page and cannot sit in a grid.
        ...(ssh.length > 1
          ? [
              {
                icon: "radio-tower",
                label: `Broadcast to ${ssh.length} device${ssh.length === 1 ? "" : "s"}`,
                run: () => openBroadcast(bulk),
              },
              "-",
            ]
          : []),
        { icon: "folder-input", label: "Move to folder…", run: () => moveAsked(bulk) },
        "-",
        {
          icon: "trash-2",
          label: `Delete ${bulk.length} devices`,
          key: "⌫",
          danger: true,
          run: () => removeMarked(bulk),
        },
      ]);
    }
    const j = shown[sel];
    // ⏎ is labelled on whichever action the device is reached by.
    const ent = (k) => (j.primary === k ? "⏎" : null);
    return showCtx(
      e.clientX,
      e.clientY,
      j.name,
      [
        ...(j.ssh
          ? [
              {
                icon: "square-terminal",
                label: "Connect",
                key: ent("ssh"),
                run: () => connect(j.name),
              },
              {
                icon: "external-link",
                label: "Open in Terminal",
                run: () => connect(j.name, true),
              },
            ]
          : []),
        ...(j.url
          ? [
              { icon: "globe", label: "Open web UI", key: ent("web"), run: () => openWeb(j.name) },
              // The browser is the only way through a certificate that needs clicking past.
              {
                icon: "external-link",
                label: "Open web UI in browser",
                run: () => invoke("open_url", { name: j.name }).catch(alertish),
              },
            ]
          : []),
        ...(j.rdp
          ? [
              {
                icon: "monitor",
                label: "Remote desktop",
                key: ent("rdp"),
                run: () => openRdp(j.name),
              },
              {
                icon: "external-link",
                label: "Remote desktop in system client",
                run: () => handOffRdp(j.name),
              },
            ]
          : []),
        // A handoff: patchbay speaks no VNC.
        ...(j.vnc
          ? [{ icon: "screen-share", label: "VNC", key: ent("vnc"), run: () => openVnc(j.name) }]
          : []),
        // sftp rides the ssh connection, so it is offered exactly where ssh is.
        ...(j.ssh
          ? [
              {
                icon: "folder",
                label: "Browse files",
                key: ent("sftp"),
                run: () => openFilesSession(j.name),
              },
            ]
          : []),
        {
          icon: "copy",
          label: "Copy ssh command",
          run: () => navigator.clipboard.writeText(j.command).catch(alertish),
        },
        "-",
        { icon: "plug", label: "Ping", run: () => openSession(j.name, "ping") },
        { icon: "waypoints", label: "Trace route", run: () => openSession(j.name, "trace") },
        "-",
        { icon: "pencil", label: "Edit…", run: () => openJack(j) },
        // Everything but the name, which is the one field a copy has to differ in.
        { icon: "copy-plus", label: "Duplicate…", run: () => openJack({ ...j, name: "" }) },
        { icon: "folder-input", label: "Move to folder…", run: () => moveAsked([j]) },
        { icon: "trash-2", label: "Delete", key: "⌫", danger: true, run: () => removeJack(j) },
      ],
      j.os ?? null,
    );
  }

  if (groupRow && groupRow.dataset.group && !groupRow.dataset.path) {
    return showCtx(e.clientX, e.clientY, "All devices", [
      { icon: "plus", label: "New device here…", run: () => openJack(null, { path: null }) },
      { icon: "folder-plus", label: "New folder…", run: () => newGroup({ path: null }) },
    ]);
  }

  if (groupRow) {
    const id = { path: groupRow.dataset.path || null };
    if (!id.path) return; // "All jacks" isn't a folder
    return showCtx(e.clientX, e.clientY, id.path, [
      { icon: "plus", label: "New device here…", run: () => openJack(null, id) },
      { icon: "folder-plus", label: "New subfolder…", run: () => newGroup(id) },
      "-",
      { icon: "pencil", label: "Rename…", run: () => renameGroup(id) },
      { icon: "trash-2", label: "Delete folder", danger: true, run: () => removeGroup(id) },
    ]);
  }

  // The file browser has its own menu.
  if (e.target.closest(".files")) {
    const s = sessions.get(activeId);
    if (!s) return;
    const row = e.target.closest("[data-fi]");
    const entry = row && s.rows[+row.dataset.fi];
    if (!entry) {
      return showCtx(e.clientX, e.clientY, s.cwd, [
        { icon: "folder-plus", label: "New folder…", run: () => makeFolder(s) },
        { icon: "rotate-cw", label: "Refresh", run: () => listFiles(s) },
        { icon: "arrow-up", label: "Up a folder", run: () => upFolder(s) },
      ]);
    }
    const path = `${s.cwd}/${entry.name}`;
    return showCtx(e.clientX, e.clientY, entry.name, [
      ...(entry.dir
        ? [{ icon: "folder-open", label: "Open", run: () => listFiles(s, path) }]
        : [{ icon: "file-pen-line", label: "Edit here", run: () => editFile(s, entry.name) }]),
      {
        icon: "download",
        label: entry.dir ? "Download folder" : "Download",
        run: () => downloadFile(s, entry.name, entry.dir),
      },
      {
        icon: "copy",
        label: "Copy path",
        run: () => navigator.clipboard.writeText(path).catch(alertish),
      },
      "-",
      { icon: "pencil", label: "Rename…", run: () => renameFile(s, entry) },
      { icon: "trash-2", label: "Delete", danger: true, run: () => removeFile(s, entry) },
    ]);
  }

  showCtx(e.clientX, e.clientY, null, [
    { icon: "plus", label: "New device…", run: () => openJack(null, group) },
    { icon: "folder-plus", label: "New folder…", run: () => newGroup({ path: null }) },
    "-",
    {
      icon: "file-pen-line",
      label: "Open config file",
      run: () => invoke("open_config").catch(alertish),
    },
    { icon: "keyboard", label: "Keyboard shortcuts", key: "?", run: () => openSettings("keys") },
  ]);
});
window.addEventListener("blur", hideCtx);
document.addEventListener("mousedown", (e) => {
  if (!e.target.closest("#ctx")) hideCtx();
});
window.addEventListener("resize", hideCtx);

// ── ask (one-line prompt) ──────────────────────────────────────────────────
let askResolve = null;
// A prompt with something to type, or a plain confirmation when `value` is null. A
// `user` of null is the one-input prompt; a string (empty included) adds the username
// field and resolves to `{ user, password }` instead.
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
  else {
    askInput.focus();
    askInput.select();
  }
  return new Promise((res) => (askResolve = res));
}
function closeAsk(v) {
  askWrap.hidden = true;
  askResolve?.(v);
  askResolve = null;
}
askForm.addEventListener("submit", (e) => {
  e.preventDefault();
  if (askBody.hidden) return closeAsk(true);
  if (askUserField.hidden) return closeAsk(askInput.value.trim() || null);
  const user = askUser.value.trim();
  if (!user) return showErr(askErr, "a username, or the desktop won't let you in");
  // Not trimmed: a space in a password is a character.
  closeAsk(askInput.value ? { user, password: askInput.value } : null);
});
$("ask-cancel").addEventListener("click", () => closeAsk(null));
askWrap.addEventListener("mousedown", (e) => {
  if (e.target === askWrap) closeAsk(null);
});

// ── update pill ────────────────────────────────────────────────────────────
// The version on offer. Closing the pill keeps it closed for this run.
let upVersion = "";
// Set once the new bundle is in place; the same button then restarts.
let upReady = false;

// Release notes from the changelog. Escaped first and marked up after, so the code
// spans are ours and everything inside them is theirs.
function notesHtml(md) {
  const items = [];
  for (const raw of md.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    if (/^[-*] /.test(line)) items.push(line.slice(2));
    // A wrapped line belongs to the bullet above it, not to a bullet of its own.
    else if (items.length) items[items.length - 1] += ` ${line}`;
    else items.push(line);
  }
  const code = (t) => esc(t).replace(/`([^`]+)`/g, "<code>$1</code>");
  return `<ul>${items.map((t) => `<li>${code(t)}</li>`).join("")}</ul>`;
}

function showUpdate(offer) {
  upVersion = offer.version;
  upText.textContent = `patchbay ${offer.version} is available`;
  upClose.innerHTML = icon("x");
  upNotes.innerHTML = offer.notes ? notesHtml(offer.notes) : "";
  upMore.hidden = !offer.notes;
  closeNotes();
  upWrap.classList.remove("leaving");
  upInstall.hidden = false;
  upWrap.hidden = false;
}

function closeNotes() {
  upNotes.hidden = true;
  upWrap.classList.remove("open");
  upMore.textContent = "What\u2019s new";
}
upMore.addEventListener("click", () => {
  if (!upNotes.hidden) return closeNotes();
  upNotes.hidden = false;
  upWrap.classList.add("open");
  upMore.textContent = "Hide";
});

// The ssh-config leftover pill, same shape as the update pill. Shown once per launch.
const slWrap = $("ssh-leftover"),
  slText = $("sl-text"),
  slMore = $("sl-more");
const slClean = $("sl-clean"),
  slClose = $("sl-close"),
  slList = $("sl-list");
let slLeftover = null;

function showSshLeftover(left) {
  slLeftover = left;
  const bits = [
    left.conf_file && "the device list",
    left.include_line && "an Include line in ~/.ssh/config",
  ].filter(Boolean);
  slText.textContent = `patchbay left ${bits.join(" and ")} behind`;
  slClose.innerHTML = icon("x");
  // Names the exact files it will touch; one of them is ~/.ssh/config.
  slList.innerHTML = `<ul class="sl-paths">
    ${left.conf_file ? `<li><b>Delete</b> <code>${esc(left.conf_path)}</code></li>` : ""}
    ${left.include_line ? `<li><b>Remove one line</b> from <code>${esc(left.config_path)}</code>: <code>Include patchbay.conf</code></li>` : ""}
  </ul>`;
  slList.hidden = true;
  slWrap.classList.remove("open", "leaving");
  slMore.textContent = "Show";
  slClean.hidden = false;
  slWrap.hidden = false;
}

function hideSshLeftover() {
  slWrap.classList.add("leaving");
  setTimeout(() => {
    slWrap.hidden = true;
    slWrap.classList.remove("leaving", "open");
  }, 220);
}

slMore.addEventListener("click", () => {
  const open = slWrap.classList.toggle("open");
  slList.hidden = !open;
  slMore.textContent = open ? "Hide" : "Show";
});
slClose.addEventListener("click", hideSshLeftover);
slClean.addEventListener("click", async () => {
  try {
    await invoke("clean_ssh_leftovers");
    flash("Cleaned up. ~/.ssh/config is back the way it was.");
    hideSshLeftover();
  } catch (e) {
    alertish(e);
  }
});

// A check someone asked for answers either way; the launch check stays quiet unless
// there is something to install. `said` is the line beside the settings button, because
// the pill is hidden behind that sheet.
async function checkUpdates(said) {
  const say = (m) => said && (said.textContent = m);
  say("Checking…");
  try {
    const offer = await invoke("update_check");
    if (offer) {
      showUpdate(offer);
      checkedNow();
      say(`${offer.version} is ready to install`);
      return;
    }
    const now = await invoke("app_version").catch(() => "");
    say(`Up to date, checked at ${checkedNow()}`);
    if (!said) flash(now ? `patchbay ${now} is the latest` : "patchbay is up to date");
  } catch (e) {
    say(`Couldn't check: ${e}`);
    if (!said) flash("Couldn't check for updates", true);
  }
}
upClose.addEventListener("click", () => (upWrap.hidden = true));
upInstall.addEventListener("click", async () => {
  // A restart takes every live session with it, so it is always a second click.
  if (upReady) return invoke("update_restart");

  upInstall.disabled = true;
  upInstall.textContent = "Downloading…";
  // The button is the progress bar; it pulses until the first percent arrives.
  upWrap.classList.add("busy");
  const stop = await listen("update:progress", ({ payload: pct }) => {
    upWrap.classList.add("determinate");
    upWrap.style.setProperty("--p", `${pct}%`);
    // The last stretch is unpacking and swapping the bundle, not downloading.
    if (pct >= 100) upInstall.textContent = "Installing…";
  }).catch(() => null);
  try {
    await invoke("update_install");
    // Back if dismissed mid-download: the restart can't be offered from a hidden pill.
    upWrap.hidden = false;
    upText.textContent = `patchbay ${upVersion} is ready`;
    upInstall.textContent = "Restart";
    upInstall.disabled = false;
    upReady = true;
  } catch (e) {
    upText.textContent = `Couldn't install: ${e}`;
    upInstall.hidden = true;
  } finally {
    stop?.();
    upWrap.classList.remove("busy", "determinate");
  }
});

// ── jack sheet ─────────────────────────────────────────────────────────────
// One way in per device; `primary` is resolved in Rust from the one field that is set.
const reachOf = (f) => f.reach.value || "ssh";

// Show only the fields the chosen way in actually needs.
function jackFields() {
  const f = jackForm.elements;
  const reach = reachOf(f);
  for (const el of jackForm.querySelectorAll("[data-need]")) {
    el.hidden = !el.dataset.need.split(" ").includes(reach);
  }
  if (reach === "rdp" && !f.rdp.value.trim()) f.rdp.value = "3389";
  if (reach === "vnc" && !f.vnc.value.trim()) f.vnc.value = "5900";
}

function openJack(j, prefillGroup) {
  editing = j?.name || null;
  editStamp = j?.stamp ?? null;
  $("sheet-title").textContent = editing ? `Edit ${editing}` : "New device";
  jfDelete.hidden = !editing;
  jfDelete.innerHTML = `${icon("trash-2")}Delete`;
  jfErr.hidden = true;
  const f = jackForm.elements;
  // `primary` is already resolved in Rust, so it is the answer even for a device that
  // still carries two ways in.
  f.reach.value = j?.primary ?? "ssh";
  f.name.value = j?.name ?? "";
  f.host.value = j?.host ?? "";
  f.user.value = j?.user ?? "";
  f.port.value = j?.port ?? "";
  f.key.value = j?.key ?? "";
  f.jump.value = j?.jump ?? "";
  f.os.value = j?.os ?? "";
  // The scheme is a control, so it cannot be typoed.
  const m = /^(https?:\/\/)(.*)$/i.exec(j?.url ?? "");
  setScheme(m ? m[1].toLowerCase() : "https://");
  f.url.value = m ? m[2] : "";
  f.rdp.value = j?.rdp ?? "";
  f.vnc.value = j?.vnc ?? "";
  f.desc.value = j?.desc ?? "";
  f.folders.value = (j?.folders ?? (prefillGroup?.path ? [prefillGroup.path] : [])).join(", ");
  f.forward.value = (j?.forward ?? []).join(", ");
  $("oschoices").innerHTML = OS_CHOICES.map((o) => `<option value="${esc(o)}">`).join("");
  renderFolderSuggestions();
  jackFields();
  sheetWrap.hidden = false;
  f.name.focus();
  jfWas = jfState();
}
// What the sheet holds, as one string: same after typing means nothing to lose.
const jfState = () => JSON.stringify([...new FormData(jackForm)]);
let jfWas = "";
const closeJack = () => {
  sheetWrap.hidden = true;
};
// Escape and the backdrop ask before discarding typed input; Save and Cancel are decisions.
async function leaveJack() {
  if (jfState() !== jfWas && !(await ask("Discard what you typed?", null, "Discard"))) return;
  closeJack();
}
const commaList = (s) =>
  s
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);

jackForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = jackForm.elements;
  const reach = reachOf(f);
  // Files ride the ssh connection, so they keep every ssh field.
  const overSsh = reach === "ssh" || reach === "sftp";
  const port = f.port.value.trim();
  const rdp = reach === "rdp" ? f.rdp.value.trim() : "";
  const vnc = reach === "vnc" ? f.vnc.value.trim() : "";
  if (overSsh && port && !/^\d+$/.test(port)) return showErr(jfErr, "ssh port has to be a number");
  if (reach === "rdp" && !/^\d+$/.test(rdp)) return showErr(jfErr, "rdp port has to be a number");
  if (reach === "vnc" && !/^\d+$/.test(vnc)) return showErr(jfErr, "vnc port has to be a number");
  try {
    await invoke("save_jack", {
      original: editing,
      jack: {
        name: f.name.value.trim(),
        host: f.host.value.trim(),
        user: f.user.value.trim() || null,
        // One way in: the other two are cleared, so a device saved with several keeps
        // only this one.
        port: overSsh && port ? +port : null,
        key: overSsh ? f.key.value.trim() || null : null,
        jump: overSsh ? f.jump.value.trim() || null : null,
        forward: reach === "ssh" ? commaList(f.forward.value) : [],
        os: f.os.value.trim() || null,
        url: reach === "web" && f.url.value.trim() ? scheme + f.url.value.trim() : null,
        rdp: rdp ? +rdp : null,
        vnc: vnc ? +vnc : null,
        ssh: overSsh ? null : false,
        // Only "sftp" needs saying: a shell and files over ssh look identical otherwise.
        primary: reach === "sftp" ? "sftp" : null,
        desc: f.desc.value.trim() || null,
        folders: commaList(f.folders.value),
        stamp: editStamp,
      },
    });
    for (const f2 of commaList(f.folders.value)) pending.delete(gkey({ path: f2 }));
    closeJack();
    await load();
  } catch (err) {
    showErr(jfErr, String(err));
    // Refused as someone else's edit: the list behind the sheet reloads, and the
    // sheet takes the new stamp, so Save a second time is a decision, not a loop.
    if (String(err).includes("changed by someone else")) {
      await load();
      editStamp = all.find((j) => j.name === editing)?.stamp ?? null;
    }
  }
});
$("jf-cancel").addEventListener("click", closeJack);
sheetWrap.addEventListener("mousedown", (e) => {
  if (e.target === sheetWrap) leaveJack();
});
jfDelete.addEventListener("click", () => {
  const j = { name: editing, stamp: editStamp };
  closeJack();
  removeJack(j);
});

function showErr(el, msg) {
  el.textContent = msg;
  el.hidden = false;
}

// ── mutations ──────────────────────────────────────────────────────────────
// A delete is offered back for a few seconds: Rust hands over the removed table and
// Undo writes it back where it was. Held here, not in the config - a wrong "yes" is
// the case, not a history.
function offerUndo(removed, failed = []) {
  const what = removed.length === 1 ? `"${removed[0].name}"` : `${removed.length} devices`;
  const but = failed.length ? ` Could not delete ${failed.join(", ")}.` : "";
  flash(`Deleted ${what}.${but}`, failed.length > 0, {
    label: "Undo",
    run: async () => {
      const stuck = [];
      for (const r of removed) {
        try {
          await invoke("restore_jack", { removed: r });
        } catch {
          stuck.push(r.name);
        }
      }
      if (stuck.length) alertish(`could not restore ${stuck.join(", ")}`);
      await load();
    },
  });
}

// The stamp is the row as it was drawn: a device a colleague has changed since the last
// poll is refused, and the list reloads so a second Delete is a decision, not a loop.
async function removeJack(j) {
  if (!(await ask(`Delete "${j.name}"? This edits your config file.`, null, "Delete"))) return;
  try {
    const removed = await invoke("delete_jack", { name: j.name, stamp: j.stamp ?? null });
    sel = 0;
    await load();
    offerUndo([removed]);
  } catch (e) {
    alertish(e);
    if (String(e).includes("changed by someone else")) await load();
  }
}

// Loops the single-device command: one path through `config.rs` is one path to get right.
async function removeMarked(js) {
  // Named, and capped so the sheet stays readable.
  const names = js
    .slice(0, 8)
    .map((j) => j.name)
    .join(", ");
  const msg =
    `Delete ${js.length} devices? This edits your config file.` +
    `\n\n${names}${js.length > 8 ? `, and ${js.length - 8} more` : ""}`;
  if (!(await ask(msg, null, "Delete"))) return;
  // One refusal doesn't abandon the rest; the ones that failed are named.
  const failed = [];
  const removed = [];
  for (const j of js) {
    try {
      removed.push(await invoke("delete_jack", { name: j.name, stamp: j.stamp ?? null }));
    } catch {
      failed.push(j.name);
    }
  }
  marked.clear();
  sel = 0;
  await load();
  if (removed.length) offerUndo(removed, failed);
  else alertish(`could not delete ${failed.join(", ")}`);
}

async function newGroup(parent) {
  const name = await ask(
    parent?.path ? `New folder inside ${parent.path}` : "New folder",
    "",
    "Create",
  );
  if (!name) return;
  const leaf = name.replace(/^\/+|\/+$/g, "");
  const id = { path: parent?.path ? `${parent.path}/${leaf}` : leaf };
  pending.set(gkey(id), id);
  openGroup(gkey({ path: id.path.split("/")[0] }));
  group = id;
  render();
}

async function renameGroup(id) {
  const to = await ask(`Rename folder ${id.path} to`, id.path.split("/").pop(), "Rename");
  if (!to) return;
  const parent = id.path.includes("/") ? id.path.slice(0, id.path.lastIndexOf("/")) : "";
  const next = { path: parent ? `${parent}/${to}` : to };
  try {
    await invoke("rename_group", { from: id.path, to: next.path });
    if (pending.delete(gkey(id))) pending.set(gkey(next), next);
    if (sameGroup(group, id)) group = next;
    await load();
  } catch (e) {
    alertish(e);
  }
}

async function removeGroup(id) {
  const n = all.filter((j) =>
    j.folders.some((f) => f === id.path || f.startsWith(id.path + "/")),
  ).length;
  const msg = `Remove folder "${id.path}" from ${n} device${n === 1 ? "" : "s"}? The devices stay.`;
  if (!(await ask(msg, null, "Remove"))) return;
  try {
    await invoke("delete_group", { path: id.path });
    pending.delete(gkey(id));
    if (sameGroup(group, id) || group?.path?.startsWith(id.path + "/")) group = null;
    await load();
  } catch (e) {
    alertish(e);
  }
}

// Put devices in a folder. Seen from inside a folder it is a move: every entry under
// that folder becomes `to`, and `to` of null takes them out of it. Seen from All
// devices it is an add, because there is nothing to move out of. Only the folders list
// is written, so a hand-written key on the device survives.
async function moveJacks(js, to) {
  const from = group?.path ?? null;
  const under = (f) => from !== null && (f === from || f.startsWith(from + "/"));
  let moved = 0;
  for (const j of js) {
    const was = j.folders;
    const next = [
      ...new Set(
        was.some(under)
          ? was.map((f) => (under(f) ? to : f)).filter((f) => f !== null)
          : to === null
            ? was
            : [...was, to],
      ),
    ];
    if (next.length === was.length && next.every((f, i) => f === was[i])) continue;
    try {
      await invoke("set_folders", { name: j.name, folders: next });
      moved++;
    } catch (e) {
      alertish(e);
      break;
    }
  }
  if (!moved) return;
  if (to !== null) {
    openGroup(gkey({ path: to.split("/")[0] }));
    pending.delete(gkey({ path: to }));
  }
  marked.clear();
  await load();
}

// "Move to folder…": the same move, typed. The list of folders is offered as you type.
async function moveAsked(js) {
  const what = js.length === 1 ? `"${js[0].name}"` : `${js.length} devices`;
  askInput.setAttribute("list", "folderlist");
  let to;
  try {
    to = await ask(`Move ${what} to folder`, group?.path ?? "", "Move");
  } finally {
    askInput.removeAttribute("list");
  }
  if (to === null) return;
  const leaf = to.replace(/^\/+|\/+$/g, "");
  await moveJacks(js, leaf || null);
}

// ── import sheet ───────────────────────────────────────────────────────────
let impFound = [];

// `where` names the source in the header: a path for an ssh config, a file name for
// a document.
async function openImport(r, where) {
  impErr.hidden = true;
  $("imp-title").textContent = `Import from ${where}`;
  $("imp-find").value = "";
  impWrap.hidden = false;

  // Devices already in the config are shown unticked, so running this twice is safe.
  impFound = r.hosts.map((h) => ({ ...h, here: all.some((j) => j.name === h.name) }));
  const fresh = impFound.filter((h) => !h.here).length;
  impNote.innerHTML =
    `<b>${impFound.length}</b> device${impFound.length === 1 ? "" : "s"} found` +
    `${fresh < impFound.length ? ` · ${impFound.length - fresh} already here` : ""}` +
    r.warnings.map((w) => `<span class="warn">${esc(w)}</span>`).join("");

  // Grouped by top folder, so a large document is a few headings rather than one list.
  const groups = new Map();
  impFound.forEach((h, i) => {
    const key = h.folders?.[0]?.split("/")[0] ?? "";
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push({ h, i });
  });
  const kind = (h) =>
    h.here
      ? "already here"
      : h.rdp
        ? "rdp"
        : h.url
          ? "web"
          : h.forward?.length
            ? `${h.forward.length} forward${h.forward.length === 1 ? "" : "s"}`
            : "ssh";

  impList.innerHTML = [...groups]
    .sort((a, b) => (a[0] === "" ? 1 : b[0] === "" ? -1 : a[0].localeCompare(b[0])))
    .map(
      ([folder, rows]) =>
        `
      <div class="imp-head" data-group="${esc(folder)}">${esc(folder || "No folder")}
        <span class="n">${rows.length}</span>
        <button type="button" data-pick="${esc(folder)}">All</button>
      </div>` +
        rows
          .map(
            ({ h, i }) => `
      <label class="imp-row" data-find="${esc(`${h.name} ${h.host} ${h.user ?? ""} ${h.folders?.join(" ") ?? ""}`.toLowerCase())}">
        <input type="checkbox" data-i="${i}"${h.here ? " disabled" : " checked"}>
        <span class="imp-name" data-tip="${esc(h.name)}">${esc(h.name)}</span>
        <span class="imp-host">${esc(h.user ? `${h.user}@${h.host}` : h.host)}${h.port ? esc(`:${h.port}`) : ""}</span>
        <span class="imp-tag">${esc(kind(h))}</span>
      </label>`,
          )
          .join(""),
    )
    .join("");
  impOk.disabled = !fresh;
}

// Hides rows rather than re-rendering, so ticks survive typing in the filter.
$("imp-find").addEventListener("input", (e) => {
  const q = e.target.value.trim().toLowerCase();
  for (const head of impList.querySelectorAll(".imp-head")) {
    let shownHere = 0;
    for (
      let el = head.nextElementSibling;
      el?.classList.contains("imp-row");
      el = el.nextElementSibling
    ) {
      const matched = !q || el.dataset.find.includes(q);
      el.classList.toggle("gone", !matched);
      shownHere += matched;
    }
    head.classList.toggle("gone", !shownHere);
  }
});

// Tick or untick one folder's rows.
impList.addEventListener("click", (e) => {
  const folder = e.target.closest("[data-pick]")?.dataset.pick;
  if (folder === undefined) return;
  e.preventDefault();
  const head = e.target.closest(".imp-head");
  const boxes = [];
  for (
    let el = head.nextElementSibling;
    el?.classList.contains("imp-row");
    el = el.nextElementSibling
  ) {
    const box = el.querySelector("input:not(:disabled)");
    if (box && !el.classList.contains("gone")) boxes.push(box);
  }
  const to = !boxes.every((b) => b.checked);
  for (const b of boxes) b.checked = to;
});

// Where an import starts. The file is read in the window and handed to Rust as text,
// so there is no dialog plugin and no new capability. The card shows progress, because
// a large document takes long enough that a silent control reads as broken.
async function impCall(card, fn) {
  const err = $("imp-perr");
  const go = card.querySelector(".go");
  err.hidden = true;
  card.dataset.state = "busy";
  go.textContent = "Reading…";
  try {
    const r = await fn();
    card.dataset.state = "done";
    go.innerHTML = `${icon("check")}${r.hosts.length} found`;
    setTimeout(() => {
      closeSettings();
      openImport(r, r.what);
    }, 550);
  } catch (e) {
    delete card.dataset.state;
    go.textContent = go.dataset.idle;
    showErr(err, String(e));
  }
}

$("imp-ssh").addEventListener("click", (e) =>
  impCall(e.currentTarget, async () => {
    const r = await invoke("ssh_hosts");
    return { ...r, what: r.path };
  }),
);

$("imp-royal").addEventListener("change", (e) => {
  const file = e.target.files?.[0];
  e.target.value = ""; // so choosing the same file twice still fires
  if (!file) return;
  impCall(e.target.closest(".source"), async () => {
    const r = await invoke("royal_hosts", { src: await file.text() });
    return { ...r, what: file.name };
  });
});

// Reset the source cards, so none claims last time's count.
function resetSources() {
  for (const c of setWrap.querySelectorAll(".source")) {
    delete c.dataset.state;
    const go = c.querySelector(".go");
    go.textContent = go.dataset.idle;
  }
}

const closeImport = () => {
  impWrap.hidden = true;
};
$("imp-cancel").addEventListener("click", closeImport);
impWrap.addEventListener("mousedown", (e) => {
  if (e.target === impWrap) closeImport();
});
// Only the rows the filter shows, so "select all" under a search means what it says.
$("imp-all").addEventListener("click", () => {
  const boxes = [...impList.querySelectorAll(".imp-row:not(.gone) input:not(:disabled)")];
  const to = !boxes.every((b) => b.checked);
  for (const b of boxes) b.checked = to;
});

impForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const picked = [...impList.querySelectorAll(".imp-row:not(.gone) input:checked")].map(
    (b) => impFound[+b.dataset.i],
  );
  if (!picked.length) return showErr(impErr, "nothing ticked to import");

  impOk.disabled = true;
  impOk.textContent = "Importing…";
  // ponytail: one write per host, so a failure names the host it was on. A bulk writer
  // if imports get large enough for the rewrites to show.
  const failed = [];
  for (const h of picked) {
    try {
      await invoke("save_jack", {
        original: null,
        jack: {
          name: h.name,
          host: h.host,
          user: h.user ?? null,
          port: h.port ?? null,
          key: h.key ?? null,
          jump: h.jump ?? null,
          folders: h.folders ?? [],
          forward: h.forward,
          rdp: h.rdp ?? null,
          url: h.url ?? null,
          // An rdp or web device is not also an ssh host; without this every import
          // opens a shell, because that is what `primary` picks first.
          ssh: h.ssh ?? null,
          os: h.os ?? null,
          desc: h.desc ?? null,
        },
      });
    } catch {
      failed.push(h.name);
    }
  }
  impOk.textContent = "Import";
  await load();

  if (!failed.length) return closeImport();
  impOk.disabled = false;
  showErr(
    impErr,
    `${picked.length - failed.length} imported, ${failed.length} refused: ${failed.join(", ")}`,
  );
});

// ── settings ───────────────────────────────────────────────────────────────
// The shortcuts page. Chords read ⌘ or Ctrl off the platform; a plain key is itself.
// A live session owns the keyboard, so the last group is the whole of what a tab
// still answers to.
function keysHtml() {
  const k = (s) => `<kbd>${esc(s)}</kbd>`;
  const groups = [
    [
      "Anywhere",
      [
        ["Search, or just start typing", [chord("k")]],
        ["New device", [chord("n")]],
        ["Settings", [chord(",")]],
        ["Open the config file", [chord("e")]],
        ["Reload the list", [chord("r")]],
        ["Previous, next tab", [`${chord("[")}`, `${chord("]")}`]],
        ["Close the tab; twice with a live session in it", [chord("w")]],
        ["This page", ["?"]],
      ],
    ],
    [
      "The list",
      [
        ["Move", ["↑", "↓"]],
        ["Fold, unfold, step through the folders", ["←", "→"]],
        ["Open, the way the device is reached", ["⏎"]],
        ["Delete", ["⌫"]],
        ["Clear the marks, then the selection", ["Esc"]],
        ["Mark several", [pickChord, "Shift-click"]],
        ["Move into a folder", ["Drag onto it"]],
      ],
    ],
    [
      "In a session",
      [
        ["Find in the scrollback", [chord("f")]],
        ["Select all, copy, paste", [chord("a"), chord("c"), chord("v")]],
        ["Text size bigger, smaller, reset", [chord("+"), chord("-"), chord("0")]],
        ...(isMac
          ? [
              ["Start, end of the line", ["⌘←", "⌘→"]],
              ["Back, forward a word", ["⌥←", "⌥→"]],
              ["Delete to the start of the line", ["⌘⌫"]],
            ]
          : []),
        ["Run it again, or reconnect, once it has ended", ["⏎"]],
      ],
    ],
  ];
  return groups
    .map(
      ([title, rows]) =>
        `<div class="keygroup"><div class="keytitle">${esc(title)}</div>${rows
          .map(
            ([what, keys]) =>
              `<div class="keyrow"><span>${esc(what)}</span><span class="keys">${keys.map(k).join("")}</span></div>`,
          )
          .join("")}</div>`,
    )
    .join("");
}

async function openSettings(pane) {
  // A pane name that names nothing would leave the sheet half empty.
  if (!setNav.querySelector(`[data-pane="${CSS.escape(String(pane ?? ""))}"]`)) pane = "devices";
  setErr.hidden = true;
  // Same selector the submit handler writes through; theme and font size are not checkboxes.
  for (const el of setForm.querySelectorAll("input[type=checkbox]")) el.checked = !!prefs[el.name];
  setForm.elements.theme.value = ["light", "dark"].includes(prefs.theme) ? prefs.theme : "system";
  setForm.elements.font_size.value = termFont();
  // Read fresh: a hand-edit between openings should show up here.
  const defs = await invoke("defaults").catch(() => ({}));
  defsStamp = defs.stamp ?? null;
  for (const k of DEFAULT_KEYS) setForm.elements[`def_${k}`].value = defs[k] ?? "";
  defsWas = defsState();
  setForm.elements.list.value = prefs.list ?? "";
  // Only worth saying while there are two files.
  const shared = cfgPath && ownPath && cfgPath !== ownPath;
  $("team-own").hidden = $("cfg-own").hidden = !shared;
  $("team-own").textContent = `Your own devices are waiting in ${ownPath}.`;
  $("cfg-own").textContent = `Settings and colours stay in ${ownPath}, this machine's own.`;
  renderSwatches();
  for (const el of setWrap.querySelectorAll("[data-icon]")) {
    if (!el.firstChild) el.innerHTML = icon(el.dataset.icon);
  }
  resetSources();
  showPane(pane);
  $("pick-list").innerHTML = `${icon("folder-open")}Choose…`;
  // Hidden rather than disabled where the OS can't host one at all: a pane that
  // could only ever say no is a pane not worth a row in the list.
  invoke("webext_supported")
    .then((ok) => ($("setnav").querySelector('[data-pane="webext"]').hidden = !ok))
    .catch(() => {});
  sayWebext();
  $("page-openconfig").innerHTML = `${icon("file-pen-line")}Open config file`;
  $("page-checkupdate").innerHTML = `${icon("rotate-cw")}Check for updates`;
  // When the last check happened is still true; its answer is not.
  $("update-said").textContent = lastChecked ? `Checked at ${lastChecked}` : "Not checked yet.";
  invoke("app_version")
    .then((v) => ($("appversion").textContent = `patchbay ${v}`))
    .catch(() => ($("appversion").textContent = "patchbay"));
  $("cfgpath").textContent = cfgPath;
  $("keylist").innerHTML = keysHtml();
  setWrap.hidden = false;
}
const closeSettings = () => {
  setWrap.hidden = true;
};

// A hidden pane is still in the form, so Save stays one submit.
function showPane(name) {
  for (const b of setNav.querySelectorAll("button")) {
    // An <svg> has no text, so a second call does not nest an icon inside an icon.
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
// Written only when touched: a save that also points at a team's list must not put
// this machine's defaults over theirs.
const defsState = () =>
  DEFAULT_KEYS.map((k) => setForm.elements[`def_${k}`].value.trim()).join("\n");
let defsWas = "";
// What [defaults] hashed to when the sheet opened. A shared list is everyone's, so the
// same refusal the jack sheet gets applies here.
let defsStamp = null;

setForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  // Over the current prefs: the sidebar width has no field here and must survive a save.
  const next = { ...prefs };
  for (const el of setForm.querySelectorAll("input[type=checkbox]")) next[el.name] = el.checked;
  next.theme = setForm.elements.theme.value;
  next.font_size = parseFloat(setForm.elements.font_size.value);
  next.list = setForm.elements.list.value.trim() || null;
  // Rust clamps it too, silently; this is the half that can say why.
  if (!(next.font_size >= 8 && next.font_size <= 32)) {
    return showErr(setErr, "terminal font size has to be between 8 and 32");
  }

  const defs = {};
  for (const k of DEFAULT_KEYS) defs[k] = setForm.elements[`def_${k}`].value.trim() || null;
  if (defs.port && !/^\d+$/.test(defs.port)) return showErr(setErr, "ssh port has to be a number");
  defs.port = defs.port ? +defs.port : null;

  try {
    const wasProbing = prefs.probe !== false;
    await invoke("save_settings", { next });
    if (defsState() !== defsWas) await invoke("save_defaults", { next: defs, stamp: defsStamp });
    prefs = next;
    listStamp = null;
    closeSettings();
    // A reload: [defaults] changes what every device shows.
    await load();
    // The throttle would otherwise hold the first sweep back by up to 30s.
    if (!wasProbing && next.probe) {
      lastProbe = 0;
      refreshProbes();
    }
  } catch (err) {
    showErr(setErr, String(err));
    // Refused as someone else's edit: the sheet takes the new stamp, so Save a second
    // time is a decision. The fields stay as typed - they are the change being kept.
    if (String(err).includes("changed by someone else")) {
      defsStamp = await invoke("defaults")
        .then((d) => d.stamp ?? null)
        .catch(() => null);
    }
  }
});
$("set-cancel").addEventListener("click", closeSettings);
// What Bitwarden for Mac has installed. Read rather than assumed, so an extension
// WKWebExtension won't take says so here instead of leaving a toggle that does nothing.
async function sayWebext() {
  const said = $("webext-said");
  said.textContent = "Reading…";
  try {
    // Started when the toggle is on, so the answer describes what is *running*
    // rather than what is merely on disk. A second start just reports.
    const on = setForm.elements.webext.checked;
    // The toggle acts at once; Save is what makes it stick for the next launch.
    if (!on) await invoke("webext_stop");
    const p = await invoke(on ? "webext_start" : "webext_inspect");
    webextRunning = p.loaded;
    renderTabs();
    const bad = p.errors.length ? ` · ${p.errors.length} warning(s)` : "";
    const state = p.loaded
      ? `running over ${p.hosts} device${p.hosts === 1 ? "" : "s"}`
      : "not running";
    said.textContent = `${p.name} ${p.version} · ${state}${bad}`;
  } catch (err) {
    said.textContent = String(err);
  }
}

setForm.elements.webext.addEventListener("change", sayWebext);

// The picker answers with null when it is cancelled, which must not clear a path
// somebody typed.
$("pick-list").addEventListener("click", async () => {
  try {
    const path = await invoke("pick_list_file");
    if (path) setForm.elements.list.value = path;
  } catch (err) {
    alertish(err);
  }
});
$("page-openconfig").addEventListener("click", () => invoke("open_config").catch(alertish));
$("page-checkupdate").addEventListener("click", () => checkUpdates($("update-said")));
setWrap.addEventListener("mousedown", (e) => {
  if (e.target === setWrap) closeSettings();
});

function renderSwatches() {
  // An os with no brand colour is drawn in the theme's faint grey, read off `:root`.
  const unset = getComputedStyle(document.body).getPropertyValue("--fg-faint").trim();
  const inUse = [...new Set(all.map((j) => osKey(j.os)).filter(Boolean))];
  const keys = [...new Set([...inUse, ...Object.keys(colors)])].sort();
  $("swatches").innerHTML = keys.length
    ? keys
        .map((k) => {
          const tint = osColor(k) ?? unset;
          const overridden = k in colors;
          // The picker edits what is stored, not the readable() nudge of it.
          const raw = colors[k] ?? tint;
          return `<span class="sw" data-os="${esc(k)}">
          <input type="color" value="${esc(/^#[0-9a-f]{6}$/i.test(raw) ? raw : unset)}">
          <span class="mark" style="color:${esc(tint)}">${osIcon(k)}</span>${esc(k)}
          ${overridden ? `<i class="reset" data-reset="${esc(k)}" data-tip="Back to the brand colour">${icon("x")}</i>` : ""}
        </span>`;
        })
        .join("")
    : `<p class="page-note">No devices have an <code>os</code> set yet.</p>`;
}

// `change`, not `input`: the picker streams a value per frame, and each is a config write.
$("swatches").addEventListener("change", async (e) => {
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
  } catch (err) {
    showErr(setErr, String(err));
  }
}

for (const r of jackForm.querySelectorAll('[name="reach"]')) {
  r.addEventListener("change", jackFields);
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
  setScheme(scheme === "https://" ? "http://" : "https://"),
);

// A plain datalist; entered folders are shown underneath as paths.
function renderFolderSuggestions() {
  const known = [...new Set(all.flatMap((j) => j.folders))].sort();
  $("folderlist").innerHTML = known.map((f) => `<option value="${esc(f)}">`).join("");
  renderCrumbs();
}

const enteredFolders = () =>
  jackForm.elements.folders.value
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);

function renderCrumbs() {
  $("crumbs").innerHTML = enteredFolders()
    .map((f) => {
      const parts = f.split("/").filter(Boolean);
      const path = parts
        .map((p, i) => `<span class="${i === parts.length - 1 ? "leafname" : ""}">${esc(p)}</span>`)
        .join(`<span class="sep">›</span>`);
      return `<span class="crumb">${path}<i class="drop" data-drop="${esc(f)}"
      data-tip="Remove">${icon("x")}</i></span>`;
    })
    .join("");
}

jackForm.elements.folders.addEventListener("input", renderCrumbs);
$("crumbs").addEventListener("click", (e) => {
  const dropped = e.target.closest("[data-drop]")?.dataset.drop;
  if (!dropped) return;
  jackForm.elements.folders.value = enteredFolders()
    .filter((f) => f !== dropped)
    .join(", ");
  renderCrumbs();
});
