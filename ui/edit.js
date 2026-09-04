// Classic script, no bundler - see the load order in ui/index.html.
// Everything that changes the config: right-click menu, prompts, the
// add/edit sheet, and the calls into config.rs.

// ── context menu ───────────────────────────────────────────────────────────
const actions = new Map();   // id -> fn, rebuilt each time the menu opens

/// `os` is the jack's, and only a jack passes one - the same mark the list row and the
/// detail pane wear, so the menu says which machine it belongs to at a glance.
function showCtx(x, y, head, items, os) {
  actions.clear();
  const mark = os === undefined ? ""
    : `<span class="ctx-os"${osColor(os) ? ` style="color:${esc(osColor(os))}"` : ""}>${osIcon(os)}</span>`;
  ctxEl.innerHTML =
    (head ? `<div class="ctx-head">${mark}<span class="ctx-name">${esc(head)}</span></div><div class="ctx-sep"></div>` : "") +
    items.map((it, i) => {
      if (it === "-") return `<div class="ctx-sep"></div>`;
      actions.set(String(i), it.run);
      const key = it.key ? `<kbd>${esc(it.key)}</kbd>` : "";
      return `<div class="ctx-item ${it.danger ? "danger" : ""}" data-a="${i}">${icon(it.icon)}${esc(it.label)}${key}</div>`;
    }).join("");
  ctxEl.hidden = false;
  // Flip back inside the window rather than overflowing it.
  const r = ctxEl.getBoundingClientRect();
  ctxEl.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  ctxEl.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
}
const hideCtx = () => { ctxEl.hidden = true; };

// The pointer takes the selection back off the keyboard, or the menu shows two
// highlighted rows and only one of them answers to Enter.
ctxEl.addEventListener("mouseover", () => {
  ctxEl.querySelector(".ctx-item.on")?.classList.remove("on");
});

ctxEl.addEventListener("click", (e) => {
  const a = e.target.closest("[data-a]")?.dataset.a;
  hideCtx();
  actions.get(a)?.();
});

// The webview's own menu is Reload / Inspect Element - never useful here.
document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  const jackRow = e.target.closest("#list .jack");
  const groupRow = e.target.closest(".group");

  if (jackRow && jackRow.dataset.i !== undefined) {
    const at = +jackRow.dataset.i;
    // Right-clicking a row that isn't picked out is a click like any other: it takes
    // the selection with it and drops the marks.
    if (!marked.has(shown[at]?.name)) marked.clear();
    select(at);
    const bulk = markedHere();
    if (bulk.length > 1) {
      const ssh = bulk.filter((x) => x.ssh);
      return showCtx(e.clientX, e.clientY, `${bulk.length} devices`, [
        // Only offered when at least two of the marks are reachable over ssh: a grid
        // of one is a session, and a grid of a NAS web UI is nothing broadcast means
        // anything for. Non-ssh marks are named in a pill rather than opened as their
        // own tabs - a webview is an OS view stacked above the page, and mixing it
        // into a grid layout is a much bigger change than this feature is.
        ...(ssh.length > 1 ? [
          { icon: "radio-tower",
            label: `Broadcast to ${ssh.length} device${ssh.length === 1 ? "" : "s"}`,
            run: () => openBroadcast(bulk) },
          "-",
        ] : []),
        { icon: "trash-2", label: `Delete ${bulk.length} devices`, key: "⌫", danger: true,
          run: () => removeMarked(bulk) },
      ]);
    }
    const j = shown[sel];
    // Enter opens whichever one of these the device is reached by, so it is labelled
    // on that row rather than on whatever happens to be first.
    const ent = (k) => (j.primary === k ? "⏎" : null);
    return showCtx(e.clientX, e.clientY, j.name, [
      ...(j.ssh ? [
        { icon: "square-terminal", label: "Connect", key: ent("ssh"), run: () => connect(j.name) },
        { icon: "external-link", label: "Open in Terminal", run: () => connect(j.name, true) },
      ] : []),
      ...(j.url ? [
        { icon: "globe", label: "Open web UI", key: ent("web"), run: () => openWeb(j.name) },
        // The handoff stays, the way Terminal and the system RDP client do - and it's
        // the only way to reach a page whose certificate needs clicking through.
        { icon: "external-link", label: "Open web UI in browser", run: () => invoke("open_url", { name: j.name }).catch(alertish) },
      ] : []),
      ...(j.rdp ? [
        { icon: "monitor", label: "Remote desktop", key: ent("rdp"), run: () => openRdp(j.name) },
        { icon: "external-link", label: "Remote desktop in system client", run: () => handOffRdp(j.name) },
      ] : []),
      // A handoff like the system RDP client, for the same reason: the viewer is the
      // one already installed, and patchbay speaks no VNC.
      ...(j.vnc ? [{ icon: "screen-share", label: "VNC", key: ent("vnc"), run: () => openVnc(j.name) }] : []),
      // sftp rides the ssh connection, so it is offered exactly where ssh is.
      ...(j.ssh ? [{ icon: "folder", label: "Browse files", key: ent("sftp"), run: () => openFilesSession(j.name) }] : []),
      { icon: "copy", label: "Copy ssh command", run: () => navigator.clipboard.writeText(j.command).catch(alertish) },
      "-",
      { icon: "plug", label: "Ping", run: () => openSession(j.name, "ping") },
      { icon: "waypoints", label: "Trace route", run: () => openSession(j.name, "trace") },
      "-",
      { icon: "pencil", label: "Edit…", run: () => openJack(j) },
      // Everything but the name, which is the one field a copy has to differ in.
      { icon: "copy-plus", label: "Duplicate…", run: () => openJack({ ...j, name: "" }) },
      { icon: "trash-2", label: "Delete", key: "⌫", danger: true, run: () => removeJack(j.name) },
    ], j.os ?? null);
  }

  if (groupRow && groupRow.dataset.group && !groupRow.dataset.path) {
    return showCtx(e.clientX, e.clientY, "All devices", [
      { icon: "plus", label: "New device here…", run: () => openJack(null, { path: null }) },
      { icon: "folder-plus", label: "New folder…", run: () => newGroup({ path: null }) },
    ]);
  }

  if (groupRow) {
    const id = { path: groupRow.dataset.path || null };
    if (!id.path) return;   // "All jacks" isn't a folder
    return showCtx(e.clientX, e.clientY, id.path, [
      { icon: "plus", label: "New device here…", run: () => openJack(null, id) },
      { icon: "folder-plus", label: "New subfolder…", run: () => newGroup(id) },
      "-",
      { icon: "pencil", label: "Rename…", run: () => renameGroup(id) },
      { icon: "trash-2", label: "Delete folder", danger: true, run: () => removeGroup(id) },
    ]);
  }

  // The file browser is a list of its own, and the app's menu is no use over it.
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
      ...(entry.dir ? [{ icon: "folder-open", label: "Open", run: () => listFiles(s, path) }] : [
        // Downloaded, opened, and put back on save - so it is above the download that
        // only does the first half of that.
        { icon: "file-pen-line", label: "Edit here", run: () => editFile(s, entry.name) },
      ]),
      { icon: "download", label: entry.dir ? "Download folder" : "Download",
        run: () => downloadFile(s, entry.name, entry.dir) },
      { icon: "copy", label: "Copy path", run: () => navigator.clipboard.writeText(path).catch(alertish) },
      "-",
      { icon: "pencil", label: "Rename…", run: () => renameFile(s, entry) },
      { icon: "trash-2", label: "Delete", danger: true, run: () => removeFile(s, entry) },
    ]);
  }

  showCtx(e.clientX, e.clientY, null, [
    { icon: "plus", label: "New device…", run: () => openJack(null, group) },
    { icon: "folder-plus", label: "New folder…", run: () => newGroup({ path: null }) },
    "-",
    { icon: "file-pen-line", label: "Open config file", run: () => invoke("open_config").catch(alertish) },
  ]);
});
window.addEventListener("blur", hideCtx);
document.addEventListener("mousedown", (e) => { if (!e.target.closest("#ctx")) hideCtx(); });
window.addEventListener("resize", hideCtx);

// ── ask (one-line prompt) ──────────────────────────────────────────────────
let askResolve = null;
// The key that answers the open question with yes, when the question was asked by a
// chord: ⌘W twice closes the session, so a confirm is a repeat and not a reach for
// the mouse. Set by the asker, cleared with the sheet.
let askAgain = null;
// A prompt when there is something to type, a plain confirmation when `value` is
// null. Pre-filling a box with the answer and then checking you typed it back is
// ceremony, not a safeguard - the button label already says what will happen.
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
function closeAsk(v) { askWrap.hidden = true; askAgain = null; askResolve?.(v); askResolve = null; }
askForm.addEventListener("submit", (e) => {
  e.preventDefault();
  if (askBody.hidden) return closeAsk(true);
  if (askUserField.hidden) return closeAsk(askInput.value.trim() || null);
  const user = askUser.value.trim();
  if (!user) return showErr(askErr, "a username, or the desktop won't let you in");
  // The password is the one field that isn't trimmed - a space in one is a character.
  closeAsk(askInput.value ? { user, password: askInput.value } : null);
});
$("ask-cancel").addEventListener("click", () => closeAsk(null));
askWrap.addEventListener("mousedown", (e) => { if (e.target === askWrap) closeAsk(null); });

// ── update pill ────────────────────────────────────────────────────────────
/// The offer to update, at the foot of the window. Closing it is an answer too:
/// it stays gone for this run, and the check at the next launch asks again.
let upVersion = "";
/// Set once the new bundle is in place, which turns the one button from the offer
/// into the restart. Nothing else about the app changes until you take it.
let upReady = false;

/// The release notes, as the changelog wrote them: bullets that wrap onto the next
/// line, and the odd `word` in backticks. Escaped first and marked up after, so the
/// code spans are ours and everything inside them is theirs.
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
  // A release with no notes offers nothing to read, so it doesn't say there is.
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

/// The ssh-config leftover pill. Same shape as the update pill so the two feel like
/// one family. Shown once per launch: dismissing hides it, and next launch nothing
/// more is done if there is nothing left to clean.
const slWrap = $("ssh-leftover"), slText = $("sl-text"), slMore = $("sl-more");
const slClean = $("sl-clean"), slClose = $("sl-close"), slList = $("sl-list");
let slLeftover = null;

function showSshLeftover(left) {
  slLeftover = left;
  const bits = [
    left.conf_file && "the device list",
    left.include_line && "an Include line in ~/.ssh/config",
  ].filter(Boolean);
  slText.textContent = `patchbay left ${bits.join(" and ")} behind`;
  slClose.innerHTML = icon("x");
  // The files, verbatim - trust and clarity both go up when the app names exactly what
  // it will touch, and one of them ("~/.ssh/config") people are careful about.
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
  setTimeout(() => { slWrap.hidden = true; slWrap.classList.remove("leaving", "open"); }, 220);
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

let msgFade = null;
/// A line at the foot of the window that takes itself away: what a check you asked
/// for found, and anything that went wrong. Its own pill rather than the update's,
/// so an error arriving mid-download can't take the Restart button off the screen.
/// Something that failed is worth a longer look than something that worked.
function flash(text, bad = false) {
  msgText.textContent = text;
  msgClose.innerHTML = icon("x");
  msgWrap.classList.toggle("bad", bad);
  msgWrap.classList.remove("leaving");
  msgWrap.hidden = false;
  clearTimeout(msgFade);
  msgFade = setTimeout(() => {
    msgWrap.classList.add("leaving");
    msgFade = setTimeout(() => {
      msgWrap.hidden = true;
      msgWrap.classList.remove("leaving");
    }, 280);
  }, bad ? 8000 : 4000);
}
msgClose.addEventListener("click", () => {
  clearTimeout(msgFade);
  msgWrap.hidden = true;
});

/// A check someone asked for, so it answers either way - unlike the one at launch,
/// which stays quiet unless there is something to install. `said` is the line beside
/// the settings button: the pill is behind that sheet and invisible while it's open,
/// so the sheet has to answer for itself.
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
  // A restart takes every live session with it, so it is never something that just
  // happens to you when a download finishes - it is this second click.
  if (upReady) return invoke("update_restart");

  upInstall.disabled = true;
  upInstall.textContent = "Downloading…";
  // The button *is* the progress bar - it fills as the bytes land, and pulses until
  // the first percent arrives, so a slow start doesn't read as a dead click.
  upWrap.classList.add("busy");
  const stop = await listen("update:progress", ({ payload: pct }) => {
    upWrap.classList.add("determinate");
    upWrap.style.setProperty("--p", `${pct}%`);
    // The last stretch is unpacking and swapping the bundle, not downloading.
    if (pct >= 100) upInstall.textContent = "Installing…";
  }).catch(() => null);
  try {
    await invoke("update_install");
    // Back if it was dismissed mid-download: the restart is the half that matters,
    // and it can't be offered from behind a pill nobody can see.
    upWrap.hidden = false;
    upText.textContent = `patchbay ${upVersion} is ready`;
    upInstall.textContent = "Restart";
    upInstall.disabled = false;
    upReady = true;
  } catch (e) {
    // Said here rather than in a sheet, for the same reason the offer wasn't one.
    upText.textContent = `Couldn't install: ${e}`;
    upInstall.hidden = true;
  } finally {
    stop?.();
    upWrap.classList.remove("busy", "determinate");
  }
});

// ── jack sheet ─────────────────────────────────────────────────────────────
/// One way in per device. There is no separate "opens on double-click" any more -
/// with a single choice the answer is the choice, and `primary` in the config is
/// whatever Rust resolves from the one field that's set.
const reachOf = (f) => f.reach.value || "ssh";

/// Show only the fields the chosen way in actually needs.
function jackFields() {
  const f = jackForm.elements;
  const reach = reachOf(f);
  for (const el of jackForm.querySelectorAll("[data-need]")) {
    el.hidden = !el.dataset.need.split(" ").includes(reach);
  }
  // Sensible starting point rather than an empty box you have to know to fill.
  if (reach === "rdp" && !f.rdp.value.trim()) f.rdp.value = "3389";
  if (reach === "vnc" && !f.vnc.value.trim()) f.vnc.value = "5900";
}

function openJack(j, prefillGroup) {
  editing = j?.name || null;
  // Which file this write lands in. An existing device stays where it is; a new one
  // goes wherever you were standing.
  $("sheet-title").textContent = editing ? `Edit ${editing}` : "New device";
  jfDelete.hidden = !editing;
  jfDelete.innerHTML = `${icon("trash-2")}Delete`;
  jfErr.hidden = true;
  const f = jackForm.elements;
  // Editing reflects what the device already has; a new one starts at terminal.
  // A device saved before this row was one choice may still carry two; `primary` is
  // already resolved in Rust, so it is the honest answer to "which one is this".
  f.reach.value = j?.primary ?? "ssh";
  f.name.value = j?.name ?? "";
  f.host.value = j?.host ?? "";
  f.user.value = j?.user ?? "";
  f.port.value = j?.port ?? "";
  f.key.value = j?.key ?? "";
  f.jump.value = j?.jump ?? "";
  f.os.value = j?.os ?? "";
  // The scheme is a control, not something to type - and not something to typo.
  const m = /^(https?:\/\/)(.*)$/i.exec(j?.url ?? "");
  setScheme(m ? m[1].toLowerCase() : "https://");
  f.url.value = m ? m[2] : "";
  f.rdp.value = j?.rdp ?? "";
  f.vnc.value = j?.vnc ?? "";
  f.desc.value = j?.desc ?? "";
  f.folders.value = (j?.folders ?? (prefillGroup?.path ? [prefillGroup.path] : [])).join(", ");
  // Only worth a control once there is somewhere else to put it.
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
const closeJack = () => { sheetWrap.hidden = true; };
/// Escape and the backdrop, the two ways out that are not a decision: a sheet with
/// something typed into it asks first. Save and Cancel are decisions and don't.
async function leaveJack() {
  if (jfState() !== jfWas && !(await ask("Discard what you typed?", null, "Discard"))) return;
  closeJack();
}
const list2 = (s) => s.split(",").map((x) => x.trim()).filter(Boolean);

jackForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = jackForm.elements;
  const reach = reachOf(f);
  // Files ride the ssh connection, so they keep every ssh field - the choice only
  // changes what a double-click does, which is the one thing `primary` records.
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
        // One way in, so the other two are cleared rather than left lying about - a
        // device edited here keeps only what it was set to, including one saved back
        // when this row allowed several.
        port: overSsh && port ? +port : null,
        key: overSsh ? f.key.value.trim() || null : null,
        jump: overSsh ? f.jump.value.trim() || null : null,
        forward: reach === "ssh" ? list2(f.forward.value) : [],
        os: f.os.value.trim() || null,
        url: reach === "web" && f.url.value.trim() ? scheme + f.url.value.trim() : null,
        rdp: rdp ? +rdp : null,
        vnc: vnc ? +vnc : null,
        ssh: overSsh ? null : false,
        // Only "sftp" needs saying: the other three are each derivable from the one
        // field that's set, but ssh-for-a-shell and ssh-for-files look identical.
        primary: reach === "sftp" ? "sftp" : null,
        desc: f.desc.value.trim() || null,
        folders: list2(f.folders.value),
      },
    });
    for (const f2 of list2(f.folders.value)) pending.delete(gkey({ path: f2 }));
    closeJack();
    await load();
  } catch (err) { showErr(jfErr, String(err)); }
});
$("jf-cancel").addEventListener("click", closeJack);
sheetWrap.addEventListener("mousedown", (e) => { if (e.target === sheetWrap) leaveJack(); });
jfDelete.addEventListener("click", () => {
  const n = editing;
  closeJack();
  removeJack(n);
});

function showErr(el, msg) { el.textContent = msg; el.hidden = false; }

// ── mutations ──────────────────────────────────────────────────────────────
async function removeJack(name) {
  if (!(await ask(`Delete "${name}"? This edits your config file.`, null, "Delete"))) return;
  try { await invoke("delete_jack", { name }); sel = 0; await load(); }
  catch (e) { alertish(e); }
}

/// Both bulk actions loop the single-device command rather than adding one of their
/// own: a config rewrite per device is fine at the sizes a list has, and one path
/// through `config.rs` is one path to get right.
async function removeMarked(js) {
  // Named, so this is a list you can check rather than a number to trust - but only
  // as far as the sheet can show without becoming a wall.
  const names = js.slice(0, 8).map((j) => j.name).join(", ");
  const msg = `Delete ${js.length} devices? This edits your config file.`
    + `\n\n${names}${js.length > 8 ? `, and ${js.length - 8} more` : ""}`;
  if (!(await ask(msg, null, "Delete"))) return;
  // One refusal doesn't abandon the rest: the ones it could delete are gone either
  // way, so the honest answer names what is still there.
  const failed = [];
  for (const j of js) {
    try { await invoke("delete_jack", { name: j.name }); }
    catch { failed.push(j.name); }
  }
  if (failed.length) alertish(`could not delete ${failed.join(", ")}`);
  marked.clear();
  sel = 0;
  await load();
}

async function newGroup(parent) {
  const name = await ask(parent?.path ? `New folder inside ${parent.path}` : "New folder", "", "Create");
  if (!name) return;
  const leaf = name.replace(/^\/+|\/+$/g, "");
  const id = { path: parent?.path ? `${parent.path}/${leaf}` : leaf };
  pending.set(gkey(id), id);
  expanded.add(gkey({ path: id.path.split("/")[0] }));
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
  } catch (e) { alertish(e); }
}

async function removeGroup(id) {
  const n = all.filter((j) =>
    j.folders.some((f) => f === id.path || f.startsWith(id.path + "/"))).length;
  const msg = `Remove folder "${id.path}" from ${n} device${n === 1 ? "" : "s"}? The devices stay.`;
  if (!(await ask(msg, null, "Remove"))) return;
  try {
    await invoke("delete_group", { path: id.path });
    pending.delete(gkey(id));
    if (sameGroup(group, id) || group?.path?.startsWith(id.path + "/")) group = null;
    await load();
  } catch (e) { alertish(e); }
}

/// Every failure the window can't put in a form: a VNC that wouldn't open, a folder
/// that wouldn't delete. It used to write into the detail pane's command box, which
/// is absent whenever a folder is selected and gone entirely under 720px - so half
/// of these went nowhere at all.
function alertish(e) {
  flash(String(e), true);
}

// ── import sheet ───────────────────────────────────────────────────────────
// The CLI prints TOML and you paste it; the window has somewhere to show the list,
// so it ticks and writes instead - through save_jack, like every other edit.
let impFound = [];

/// Whatever was found, from either source. `where` is what to call it in the header -
/// an ssh config has a path, a document handed in through the window has a name.
async function openImport(r, where) {
  impErr.hidden = true;
  $("imp-title").textContent = `Import from ${where}`;
  $("imp-find").value = "";
  impWrap.hidden = false;

  // A device already in the config is shown but not ticked, so running this twice is
  // safe and you can see what it would have added.
  impFound = r.hosts.map((h) => ({ ...h, here: all.some((j) => j.name === h.name) }));
  const fresh = impFound.filter((h) => !h.here).length;
  impNote.innerHTML =
    `<b>${impFound.length}</b> device${impFound.length === 1 ? "" : "s"} found` +
    `${fresh < impFound.length ? ` · ${impFound.length - fresh} already here` : ""}` +
    r.warnings.map((w) => `<span class="warn">${esc(w)}</span>`).join("");

  // Grouped by the folder they came out of. A document is a customer per folder, and
  // a hundred rows in one run is a list nobody reads - as headings it is twenty
  // groups you can tick one at a time.
  const groups = new Map();
  impFound.forEach((h, i) => {
    const key = h.folders?.[0]?.split("/")[0] ?? "";
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push({ h, i });
  });
  const kind = (h) => h.here ? "already here"
    : h.rdp ? "rdp" : h.url ? "web"
    : h.forward?.length ? `${h.forward.length} forward${h.forward.length === 1 ? "" : "s"}` : "ssh";

  impList.innerHTML = [...groups]
    .sort((a, b) => (a[0] === "" ? 1 : b[0] === "" ? -1 : a[0].localeCompare(b[0])))
    .map(([folder, rows]) => `
      <div class="imp-head" data-group="${esc(folder)}">${esc(folder || "No folder")}
        <span class="n">${rows.length}</span>
        <button type="button" data-pick="${esc(folder)}">All</button>
      </div>` + rows.map(({ h, i }) => `
      <label class="imp-row" data-find="${esc(`${h.name} ${h.host} ${h.user ?? ""} ${h.folders?.join(" ") ?? ""}`.toLowerCase())}">
        <input type="checkbox" data-i="${i}"${h.here ? " disabled" : " checked"}>
        <span class="imp-name" data-tip="${esc(h.name)}">${esc(h.name)}</span>
        <span class="imp-host">${esc(h.user ? `${h.user}@${h.host}` : h.host)}${h.port ? `:${h.port}` : ""}</span>
        <span class="imp-tag">${esc(kind(h))}</span>
      </label>`).join("")).join("");
  impOk.disabled = !fresh;
}

/// Hides rows rather than re-rendering them, so a tick survives typing in the filter -
/// and a heading goes with the last of its rows.
$("imp-find").addEventListener("input", (e) => {
  const q = e.target.value.trim().toLowerCase();
  for (const head of impList.querySelectorAll(".imp-head")) {
    let shownHere = 0;
    for (let el = head.nextElementSibling; el?.classList.contains("imp-row"); el = el.nextElementSibling) {
      const hit = !q || el.dataset.find.includes(q);
      el.classList.toggle("gone", !hit);
      shownHere += hit;
    }
    head.classList.toggle("gone", !shownHere);
  }
});

/// One folder's worth, which is one customer's worth.
impList.addEventListener("click", (e) => {
  const folder = e.target.closest("[data-pick]")?.dataset.pick;
  if (folder === undefined) return;
  e.preventDefault();
  const head = e.target.closest(".imp-head");
  const boxes = [];
  for (let el = head.nextElementSibling; el?.classList.contains("imp-row"); el = el.nextElementSibling) {
    const box = el.querySelector("input:not(:disabled)");
    if (box && !el.classList.contains("gone")) boxes.push(box);
  }
  const to = !boxes.every((b) => b.checked);
  for (const b of boxes) b.checked = to;
});

/// The one place an import starts. A file is read here in the window and handed to
/// Rust as text, so there is no dialog plugin, no new capability, and the app only
/// ever sees a document somebody chose.
///
/// The card carries the state rather than a pill: reading a two-hundred device document
/// takes long enough that a control which does nothing visible reads as broken, and the
/// count it lands on is the answer to "did that work".
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
    // Long enough to read, short enough that the sheet isn't sitting there afterwards.
    setTimeout(() => { closeSettings(); openImport(r, r.what); }, 550);
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
  }));

$("imp-royal").addEventListener("change", (e) => {
  const file = e.target.files?.[0];
  e.target.value = "";   // so choosing the same file twice still fires
  if (!file) return;
  impCall(e.target.closest(".source"), async () => {
    const r = await invoke("royal_hosts", { src: await file.text() });
    return { ...r, what: file.name };
  });
});

/// Back to "Choose a file…" whenever the pane is opened again, so a card never sits
/// there claiming a count from last time.
function resetSources() {
  for (const c of setWrap.querySelectorAll(".source")) {
    delete c.dataset.state;
    const go = c.querySelector(".go");
    go.textContent = go.dataset.idle;
  }
}

const closeImport = () => { impWrap.hidden = true; };
$("imp-cancel").addEventListener("click", closeImport);
impWrap.addEventListener("mousedown", (e) => { if (e.target === impWrap) closeImport(); });
/// Everything the filter is currently showing, so "select all" under a search means
/// what it says rather than quietly ticking the hundred rows you filtered away.
$("imp-all").addEventListener("click", () => {
  const boxes = [...impList.querySelectorAll(".imp-row:not(.gone) input:not(:disabled)")];
  const to = !boxes.every((b) => b.checked);
  for (const b of boxes) b.checked = to;
});

impForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const picked = [...impList.querySelectorAll(".imp-row:not(.gone) input:checked")]
    .map((b) => impFound[+b.dataset.i]);
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
          folders: h.folders ?? [], forward: h.forward,
          rdp: h.rdp ?? null, url: h.url ?? null,
          os: h.os ?? null, desc: h.desc ?? null,
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

// ── settings ───────────────────────────────────────────────────────────────
async function openSettings(pane) {
  // Defended here as well as at the call sites: a pane name that names nothing would
  // leave the sheet open with an empty right-hand side and no way to tell why.
  if (!setNav.querySelector(`[data-pane="${CSS.escape(String(pane ?? ""))}"]`)) pane = "devices";
  setErr.hidden = true;
  // The same selector the submit handler writes back through, so a setting that is
  // not a checkbox - theme, font size - is read here rather than assigned `.checked`.
  for (const el of setForm.querySelectorAll("input[type=checkbox]")) el.checked = !!prefs[el.name];
  setForm.elements.theme.value = ["light", "dark"].includes(prefs.theme) ? prefs.theme : "system";
  setForm.elements.font_size.value = termFont();
  // Read fresh rather than from state: nothing else in the app needs [defaults],
  // and a hand-edit between openings should show up here.
  const defs = await invoke("defaults").catch(() => ({}));
  for (const k of DEFAULT_KEYS) setForm.elements[`def_${k}`].value = defs[k] ?? "";
  renderSwatches();
  for (const el of setWrap.querySelectorAll("[data-icon]")) {
    if (!el.firstChild) el.innerHTML = icon(el.dataset.icon);
  }
  resetSources();
  showPane(pane);
  $("page-openconfig").innerHTML = `${icon("file-pen-line")}Open config file`;
  $("page-checkupdate").innerHTML = `${icon("rotate-cw")}Check for updates`;
  // Last time's answer is not this time's, and the sheet outlives one opening - but
  // when the check happened is still true, and is the thing this pane is asked.
  $("update-said").textContent = lastChecked ? `Checked at ${lastChecked}` : "Not checked yet.";
  invoke("app_version")
    .then((v) => ($("appversion").textContent = `patchbay ${v}`))
    .catch(() => ($("appversion").textContent = "patchbay"));
  // Already in state from the first load; a second ask is a second way to be blank.
  $("cfgpath").textContent = cfgPath;
  setWrap.hidden = false;
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
  // Over the current prefs, not from nothing: the sidebar width has no field here,
  // and a save that forgot it handed Rust its default and snapped the column back.
  const next = { ...prefs };
  for (const el of setForm.querySelectorAll("input[type=checkbox]")) next[el.name] = el.checked;
  next.theme = setForm.elements.theme.value;
  next.font_size = parseFloat(setForm.elements.font_size.value);
  // Rust clamps it too, but silently: this is the half that can say why.
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
$("page-openconfig").addEventListener("click", () => invoke("open_config").catch(alertish));
$("page-checkupdate").addEventListener("click", () => checkUpdates($("update-said")));
setWrap.addEventListener("mousedown", (e) => { if (e.target === setWrap) closeSettings(); });


function renderSwatches() {
  const inUse = [...new Set(all.map((j) => osKey(j.os)).filter(Boolean))];
  const keys = [...new Set([...inUse, ...Object.keys(colors)])].sort();
  $("swatches").innerHTML = keys.length
    ? keys.map((k) => {
        const shown = osColor(k) ?? "#8b8b95";
        const overridden = k in colors;
        // The picker edits what is stored, not what is painted: `osColor` nudges a
        // colour until it is readable here, and offering that back saves the nudge.
        const raw = colors[k] ?? shown;
        return `<span class="sw" data-os="${esc(k)}">
          <input type="color" value="${esc(/^#[0-9a-f]{6}$/i.test(raw) ? raw : "#8b8b95")}">
          <span class="mark" style="color:${esc(shown)}">${osIcon(k)}</span>${esc(k)}
          ${overridden ? `<i class="reset" data-reset="${esc(k)}" data-tip="Back to the brand colour">${icon("x")}</i>` : ""}
        </span>`;
      }).join("")
    : `<p class="page-note">No devices have an <code>os</code> set yet.</p>`;
}

// `change`, not `input`: the picker streams a value per frame while you drag it, and
// each one of those is a config write.
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
  } catch (err) { showErr(setErr, String(err)); }
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
