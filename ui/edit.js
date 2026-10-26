// Everything that changes the config: context menu, prompts, the device sheet, settings
// and import. Classic script; see the load order in ui/index.html.
//
// A device is reached one way. The sheet's row is a radio, and saving clears the other two - so a
// device edited there keeps only what it was set to, including one written back when the row
// allowed several. `primary` is still *read* (old configs, and hand-edited ones can still set two),
// and it is still resolved so an option pointing at something the device no longer has falls back
// rather than doing nothing - but the window no longer writes it, because with one field set the
// answer is derivable. That resolution is `primary()` in `patchbay.rs`, with tests: `primary` only
// picks the default action, so a device with a `url` *and* ssh still has both.
//
// Delete hands back what it removed. `delete_jack` returns a `Removed` - the table as a one-table
// TOML document plus the position it had - and the pill's Undo sends it to `restore_jack`, which
// makes room at that position (`make_room`: positions are renumbered by every parse, so the old
// slot is taken by whatever followed). The comments above a deleted jack are rehomed to the next
// table and *stay there* through an undo: stale is visible, duplicated is not. Held in the window
// for the life of the pill, never in the config - a wrong "yes" is the case, not a history.

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
      : "") + ctxRows(items, "");
  // Asked before the menu is up, because once it is the page counts as covered.
  const web = webViewRect();
  ctxEl.hidden = false;
  // Flip back inside the window rather than overflowing it.
  const r = ctxEl.getBoundingClientRect();
  let left = Math.min(x, innerWidth - r.width - 8);
  // A menu opened beside a web tab stays beside it: the page is an OS view, and one
  // pixel of menu over it means the whole page has to go while the menu is up.
  if (web && x < web.left) left = Math.max(4, Math.min(left, web.left - r.width - 4));
  ctxEl.style.left = `${left}px`;
  ctxEl.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
  // A submenu flies out to the right unless that runs off the window or over a web tab,
  // then to the left, and with room on neither side it lies over the menu, against the
  // edge. Lifted to stay above the bottom edge. Measured here because hidden it has no box.
  for (const sub of ctxEl.querySelectorAll(".ctx-sub")) {
    sub.style.display = "block";
    const s = sub.getBoundingClientRect();
    const item = sub.parentElement.getBoundingClientRect();
    sub.style.display = "";
    const edge = web && x < web.left ? web.left : innerWidth;
    const at =
      item.right + s.width <= edge - 4
        ? item.right
        : item.left - s.width >= 4
          ? item.left - s.width
          : Math.max(4, edge - 4 - s.width);
    sub.style.left = `${at - item.left}px`;
    sub.style.top = `${Math.min(-6, innerHeight - 8 - item.top - s.height)}px`;
  }
}
function ctxRows(items, at) {
  return items
    .map((it, i) => {
      if (it === "-") return `<div class="ctx-sep" role="separator"></div>`;
      const id = at + i;
      if (it.sub) {
        return `<div class="ctx-item has-sub" role="menuitem" aria-haspopup="menu" tabindex="-1">${icon(it.icon)}${esc(it.label)}<span class="ctx-more">${icon("chevron-right")}</span><div class="ctx-sub" role="menu">${ctxRows(it.sub, `${id}.`)}</div></div>`;
      }
      actions.set(id, it.run);
      const key = it.key ? `<kbd>${esc(it.key)}</kbd>` : "";
      return `<div class="ctx-item ${it.danger ? "danger" : ""}" data-a="${id}" role="menuitem" tabindex="-1">${icon(it.icon)}${esc(it.label)}${key}</div>`;
    })
    .join("");
}
function copyItems(j) {
  const copy = (text) => () => navigator.clipboard.writeText(text).catch(alertish);
  return [
    ...(j.host ? [{ icon: "copy", label: j.host, run: copy(j.host) }] : []),
    ...(j.user
      ? [{ icon: "copy", label: `${j.user}@${j.host}`, run: copy(`${j.user}@${j.host}`) }]
      : []),
    ...(j.url ? [{ icon: "copy", label: "Web UI address", run: copy(j.url) }] : []),
    ...(j.mac ? [{ icon: "copy", label: j.mac, run: copy(j.mac) }] : []),
    // Only for a device ssh reaches: anything else's command line is the error saying so.
    ...(j.ssh ? [{ icon: "copy", label: "ssh command", run: copy(j.command) }] : []),
  ];
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
  if (a === undefined && e.target.closest(".has-sub")) return;
  hideCtx();
  actions.get(a)?.();
});

// The webview's own menu is Reload / Inspect Element.
document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  const jackRow = e.target.closest("#list .jack, #tree .jack");
  const groupRow = e.target.closest(".group");

  if (jackRow && jackRow.dataset.i !== undefined) {
    let at = +jackRow.dataset.i;
    if (at < 0) at = revealDevice(jackRow.dataset.name);
    if (at < 0) return;
    // Right-clicking an unmarked row is an ordinary click: it selects and drops the marks.
    if (!marked.has(shown[at]?.name)) marked.clear();
    select(at, true);
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
        ...vaultMoves(bulk),
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
    const front = activeId !== null && !sessions.get(activeId)?.bcast;
    return showCtx(
      e.clientX,
      e.clientY,
      j.label,
      [
        // The sidebar stays while a tab is open, so this is a split from the list.
        ...(front
          ? [
              {
                icon: "columns-2",
                label: "Open beside the tab in front",
                run: () => openBeside(j.name),
              },
              "-",
            ]
          : []),
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
        ...(j.vnc
          ? [
              { icon: "screen-share", label: "VNC", key: ent("vnc"), run: () => openVnc(j.name) },
              {
                icon: "external-link",
                label: "VNC in system viewer",
                run: () => handOffVnc(j.name),
              },
            ]
          : []),
        ...(j.telnet
          ? [
              {
                icon: "terminal",
                label: "Telnet",
                key: ent("telnet"),
                run: () => openLine(j.name, "telnet"),
              },
            ]
          : []),
        ...(j.serial
          ? [
              {
                icon: "cable",
                label: "Serial console",
                key: ent("serial"),
                run: () => openLine(j.name, "serial"),
              },
            ]
          : []),
        // Always offered: asking the keychain whether there is one is itself a prompt on
        // macOS, and a right-click must not raise one. One item forgets RDP's and VNC's.
        ...(j.rdp || j.vnc
          ? [{ icon: "key-round", label: "Forget password", run: () => forgetRdp(j.name) }]
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
              { icon: "key-round", label: "Install my key", run: () => openSession(j.name, "key") },
            ]
          : []),
        { icon: "copy", label: "Copy", sub: copyItems(j) },
        "-",
        { icon: "plug", label: "Ping", run: () => openSession(j.name, "ping") },
        { icon: "waypoints", label: "Trace route", run: () => openSession(j.name, "trace") },
        "-",
        { icon: "pencil", label: "Edit…", run: () => openJack(j) },
        // Everything but the name, which is the one field a copy has to differ in.
        { icon: "copy-plus", label: "Duplicate…", run: () => openJack({ ...j, name: "" }) },
        { icon: "folder-input", label: "Move to folder…", run: () => moveAsked([j]) },
        ...vaultMoves([j]),
        { icon: "trash-2", label: "Delete", key: "⌫", danger: true, run: () => removeJack(j) },
      ],
      j.os ?? null,
    );
  }

  if (groupRow) {
    const id = idOf(groupRow);
    // A vault's own row: somewhere to put things, not a folder to rename.
    if (!id.path) {
      return showCtx(e.clientX, e.clientY, groupLabelOf(id), [
        { icon: "plus", label: "New device here…", run: () => openJack(null, id) },
        { icon: "folder-plus", label: "New folder…", run: () => newGroup(id) },
      ]);
    }
    if (!markedFolders.has(gkey(id))) unmarkFolders();
    const fs = foldersMarked();
    if (fs.length > 1) {
      return showCtx(e.clientX, e.clientY, `${fs.length} folders`, [
        {
          icon: "trash-2",
          label: `Remove ${fs.length} folders`,
          key: "⌫",
          danger: true,
          run: () => removeFolders(fs),
        },
      ]);
    }
    return showCtx(e.clientX, e.clientY, id.path, [
      { icon: "plus", label: "New device here…", run: () => openJack(null, id) },
      { icon: "folder-plus", label: "New subfolder…", run: () => newGroup(id) },
      {
        icon: "settings",
        label: "Settings for devices in here…",
        run: () => {
          pickGroup(id, false);
          $("gdefs")?.querySelector("input")?.focus();
        },
      },
      "-",
      { icon: "pencil", label: "Rename…", run: () => renameGroup(id) },
      { icon: "trash-2", label: "Delete folder", danger: true, run: () => removeGroup(id) },
    ]);
  }

  // A split's chip: which way its panes lie, and the way back to tabs of their own.
  const splitChip = e.target.closest("#tabs .tab[data-sid]");
  if (splitChip) {
    const sid = +splitChip.dataset.sid;
    const g = [...sessions.values()].find((s) => s.split?.sid === sid)?.split;
    if (!g) return;
    return showCtx(e.clientX, e.clientY, `${g.ids.length} panes`, [
      {
        icon: g.dir === "row" ? "rows-2" : "columns-2",
        label: g.dir === "row" ? "Stack the panes" : "Panes side by side",
        run: () => {
          g.dir = g.dir === "row" ? "col" : "row";
          showTab();
          renderTabs();
        },
      },
      { icon: "copy", label: "Separate into tabs", run: () => separateSplit(sid) },
      "-",
      { icon: "x", label: "Close all", run: () => closeSplit(sid) },
    ]);
  }

  // A tab: what can open beside it, and for a shell the snippets, which live in the
  // palette and need a menu to be found.
  const tab = e.target.closest("#tabs .tab[data-id]");
  const onTab = tab && sessions.get(+tab.dataset.id);
  if (onTab) {
    const front = (then) => () => {
      activeId = onTab.id;
      showTab();
      renderTabs();
      then();
    };
    const shell = onTab.kind === "term" && !onTab.task && !onTab.dead;
    return showCtx(e.clientX, e.clientY, labelOf(onTab.name), [
      ...(shell
        ? [
            {
              icon: "text-cursor-input",
              label: "Snippets…",
              key: chord("k"),
              run: front(() => openPalette()),
            },
          ]
        : []),
      ...(onTab.bcast
        ? []
        : [
            {
              icon: "columns-2",
              label: "Open beside…",
              key: chord("d"),
              run: front(() => openPalette("", true)),
            },
          ]),
      "-",
      { icon: "x", label: "Close", key: chord("w"), run: () => closeSession(onTab.id) },
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
    {
      icon: "folder-plus",
      label: "New folder…",
      run: () => newGroup({ path: null, vault: group?.vault }),
    },
    "-",
    {
      icon: "file-pen-line",
      label: "Open config file",
      run: () => invoke("open_config").catch(alertish),
    },
    { icon: "keyboard", label: "Keyboard shortcuts", key: "?", run: () => openSettings("keys") },
  ]);
});
// "Move to Team" or "Move to Mine", for whichever of the devices are not there yet.
// Nothing with one vault: there is nowhere else to be.
function vaultMoves(js) {
  return Object.keys(VAULTS)
    .filter((v) => js.some((j) => j.vault && j.vault !== v))
    .map((v) => ({
      icon: VAULTS[v][0],
      label: `Move to ${VAULTS[v][1]}`,
      run: () => moveVault(js.filter((j) => j.vault !== v)),
    }));
}
const groupLabelOf = (id) => id.path ?? VAULTS[id.vault]?.[1] ?? "All devices";

window.addEventListener("blur", hideCtx);
document.addEventListener("mousedown", (e) => {
  if (!e.target.closest("#ctx")) hideCtx();
});
window.addEventListener("resize", hideCtx);

// ── ask (one-line prompt) ──────────────────────────────────────────────────
let askResolve = null;
// A prompt with something to type, or a plain confirmation when `value` is null. A
// `user` of null is the one-input prompt; a string (empty included) adds the username
// field and resolves to `{ user, password, remember }` instead; `remember` of null hides
// that tick box (and resolves null), a boolean is how it starts. `saved` are accounts
// this machine has a password for: offered as chips under the username, and one of them
// with the password left empty resolves `password: null`, the saved one.
let askSaved = [];
const askIsSaved = () =>
  askSaved.some((u) => u.toLowerCase() === askUser.value.trim().toLowerCase());
function askHint() {
  const saved = askIsSaved();
  // A chip for the account already typed only repeats the field.
  const typed = askUser.value.trim().toLowerCase();
  for (const b of $("ask-saved").children) b.hidden = b.dataset.saved.toLowerCase() === typed;
  askInput.placeholder = saved ? "saved on this machine" : "";
  // Typed over a saved one, the new password replaces it rather than forgetting it.
  $("ask-remember").checked = saved;
}
askUser.addEventListener("input", askHint);
$("ask-saved").addEventListener("click", (e) => {
  const u = e.target.closest("[data-saved]")?.dataset.saved;
  if (u === undefined) return;
  askUser.value = u;
  askInput.value = "";
  askHint();
  askInput.focus();
});
function ask(
  title,
  value = "",
  okLabel = "OK",
  type = "text",
  user = null,
  remember = null,
  saved = [],
) {
  const confirming = value === null;
  $("ask-title").textContent = title;
  $("ask-facts").hidden = true;
  askBody.hidden = confirming;
  askInput.type = type;
  askInput.value = confirming ? "" : value;
  askUserField.hidden = user === null;
  askUser.value = user ?? "";
  askLabel.hidden = user === null;
  askLabel.textContent = "Password";
  // VNC asks a password with no account, and can still remember it.
  $("ask-remember-field").hidden = remember === null;
  $("ask-remember").checked = !!remember;
  askSaved = saved;
  $("ask-saved").innerHTML = saved
    .map((u) => `<button type="button" class="crumb" data-saved="${esc(u)}">${esc(u)}</button>`)
    .join("");
  askInput.placeholder = "";
  if (saved.length) askHint();
  $("ask-vault").hidden = user === null || !webextRunning;
  $("ask-vault").innerHTML = `${icon("key-round")}Vault`;
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
// A yes/no with rows to read first: [label, value, extra class] each.
function askFacts(title, note, rows, okLabel) {
  const answer = ask(title, null, okLabel);
  const facts = $("ask-facts");
  facts.innerHTML =
    (note ? `<p class="ask-note">${esc(note)}</p>` : "") +
    rows.map(([k, v, cls]) => `<dt>${esc(k)}</dt><dd class="${cls ?? ""}">${esc(v)}</dd>`).join("");
  facts.hidden = false;
  return answer;
}
function closeAsk(v) {
  askWrap.hidden = true;
  askResolve?.(v);
  askResolve = null;
}
askForm.addEventListener("submit", (e) => {
  e.preventDefault();
  if (askBody.hidden) return closeAsk(true);
  if (askUserField.hidden && !$("ask-remember-field").hidden) {
    const password = askInput.value;
    return closeAsk(password ? { password, remember: $("ask-remember").checked } : null);
  }
  // Not trimmed for a password, as below.
  const v = askInput.type === "password" ? askInput.value : askInput.value.trim();
  if (askUserField.hidden) return closeAsk(v || null);
  const user = askUser.value.trim();
  if (!user) return showErr(askErr, "a username, or the desktop won't let you in");
  // Not trimmed: a space in a password is a character.
  if (!askInput.value && askIsSaved()) return closeAsk({ user, password: null, remember: null });
  const remember = $("ask-remember-field").hidden ? null : $("ask-remember").checked;
  closeAsk(askInput.value ? { user, password: askInput.value, remember } : null);
});
$("ask-cancel").addEventListener("click", () => closeAsk(null));
$("ask-vault").addEventListener("click", (e) => bitwardenKey(e.currentTarget, true));
askWrap.addEventListener("mousedown", (e) => {
  if (e.target === askWrap) closeAsk(null);
});

// ── update pill ────────────────────────────────────────────────────────────
// The version on offer. Closing the pill keeps it closed for this run.
let upVersion = "";
// Set once the new bundle is in place; the same button then restarts.
let upReady = false;
// An offer found from inside Settings. The pill sits under every overlay, so showing it
// there puts the only Install button behind the sheet's blur, unclickable.
let upWaiting = null;

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
// the pill is hidden behind that sheet - and an offer found there waits for it to close
// rather than showing an Install button nobody can reach.
async function checkUpdates(said) {
  const say = (m) => said && (said.textContent = m);
  say("Checking…");
  try {
    const offer = await invoke("update_check");
    if (offer) {
      checkedNow();
      say(`${offer.version} is ready - close Settings to install it`);
      if (said) upWaiting = offer;
      else showUpdate(offer);
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
  if (reach === "telnet" && !f.telnet.value.trim()) f.telnet.value = "23";
}

function openJack(j, prefillGroup) {
  editing = j?.name || null;
  editStamp = j?.stamp ?? null;
  $("sheet-title").textContent = editing ? `Edit ${j.label}` : "New device";
  jfDelete.hidden = !editing;
  jfDelete.innerHTML = `${icon("trash-2")}Delete`;
  jfErr.hidden = true;
  const f = jackForm.elements;
  // Where it is, or where you are looking; the team's when neither says, as before.
  $("jf-vault").hidden = !vaults.two;
  f.vault.value = j?.vault ?? prefillGroup?.vault ?? "team";
  // `primary` is already resolved in Rust, so it is the answer even for a device that
  // still carries two ways in.
  f.reach.value = j?.primary ?? "ssh";
  f.name.value = j?.label ?? "";
  f.host.value = j?.host ?? "";
  // An inherited value is shown, not filled in: saved back, it would be written onto
  // the device and the folder or [defaults] it came from would stop applying to it.
  for (const k of ["user", "port", "key", "jump"]) {
    const el = f[k];
    el.dataset.ph ??= el.placeholder;
    const who = j?.from?.[k];
    el.value = who ? "" : (j?.[k] ?? "");
    el.placeholder = who
      ? `${j[k]} from ${who === "defaults" ? "[defaults]" : who}`
      : el.dataset.ph;
  }
  f.os.value = j?.os ?? "";
  // The scheme is a control, so it cannot be typoed.
  const m = /^(https?:\/\/)(.*)$/i.exec(j?.url ?? "");
  setScheme(m ? m[1].toLowerCase() : "https://");
  f.url.value = m ? m[2] : "";
  f.rdp.value = j?.rdp ?? "";
  f.vnc.value = j?.vnc ?? "";
  f.telnet.value = j?.telnet ?? "";
  f.serial.value = j?.serial ?? "";
  // A speed the menu doesn't list (a hand-edited 2400) is kept rather than reset.
  const baud = String(j?.baud ?? 9600);
  if (![...f.baud.options].some((o) => o.value === baud)) f.baud.add(new Option(baud, baud));
  f.baud.value = baud;
  f.desc.value = j?.desc ?? "";
  f.mac.value = j?.mac ?? "";
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
  const telnet = reach === "telnet" ? f.telnet.value.trim() : "";
  const serial = reach === "serial" ? f.serial.value.trim() : "";
  if (overSsh && port && !/^\d+$/.test(port)) return showErr(jfErr, "ssh port has to be a number");
  if (reach === "rdp" && !/^\d+$/.test(rdp)) return showErr(jfErr, "rdp port has to be a number");
  if (reach === "vnc" && !/^\d+$/.test(vnc)) return showErr(jfErr, "vnc port has to be a number");
  if (reach === "telnet" && !/^\d+$/.test(telnet))
    return showErr(jfErr, "telnet port has to be a number");
  if (reach === "serial" && !serial.startsWith("/dev/"))
    return showErr(jfErr, "a serial device is a /dev/ path, like /dev/cu.usbserial-1410");
  try {
    await invoke("save_jack", {
      original: editing,
      jack: {
        name: f.name.value.trim(),
        // A console cable has no address, so a serial device keeps none.
        host: reach === "serial" ? "" : f.host.value.trim(),
        user: f.user.value.trim() || null,
        // One way in: the other two are cleared, so a device saved with several keeps
        // only this one.
        port: overSsh && port ? +port : null,
        key: overSsh ? f.key.value.trim() || null : null,
        jump: overSsh || reach === "telnet" ? f.jump.value.trim() || null : null,
        forward: reach === "ssh" ? commaList(f.forward.value) : [],
        os: f.os.value.trim() || null,
        url: reach === "web" && f.url.value.trim() ? scheme + f.url.value.trim() : null,
        rdp: rdp ? +rdp : null,
        vnc: vnc ? +vnc : null,
        telnet: telnet ? +telnet : null,
        serial: serial || null,
        baud: serial ? +f.baud.value : null,
        ssh: overSsh ? null : false,
        // Only "sftp" needs saying: a shell and files over ssh look identical otherwise.
        primary: reach === "sftp" ? "sftp" : null,
        desc: f.desc.value.trim() || null,
        mac: f.mac.value.trim() || null,
        folders: commaList(f.folders.value),
        stamp: editStamp,
      },
      vault: vaults.two ? f.vault.value : null,
    });
    const vault = vaults.two ? f.vault.value : undefined;
    for (const f2 of commaList(f.folders.value)) pending.delete(gkey({ path: f2, vault }));
    closeJack();
    // A device just written has no dot until the next sweep; the throttle would hold
    // that back by up to 30s.
    lastProbe = 0;
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
let undoable = [];
let undoPill = null;
function offerUndo(removed, failed = []) {
  // A delete while the pill is still up joins it, so one Undo takes back both.
  if (undoPill?.isConnected) undoPill.remove();
  else undoable = [];
  undoable.push(...removed);
  const all = undoable;
  const what = all.length === 1 ? `"${all[0].name}"` : `${all.length} devices`;
  const but = failed.length ? ` Could not delete ${failed.join(", ")}.` : "";
  const run = async () => {
    const stuck = [];
    // Newest first: each position was read from the file with the earlier ones gone.
    for (const r of [...all].reverse()) {
      try {
        await invoke("restore_jack", { removed: r });
      } catch {
        stuck.push(r.name);
      }
    }
    if (stuck.length) alertish(`could not restore ${stuck.join(", ")}`);
    await load();
  };
  undoPill = flash(`Deleted ${what}.${but}`, failed.length > 0, { label: "Undo", run });
}

// The stamp is the row as it was drawn: a device a colleague has changed since the last
// poll is refused, and the list reloads so a second Delete is a decision, not a loop.
async function removeJack(j) {
  if (!(await ask(`Delete "${j.label}"? This edits your config file.`, null, "Delete"))) return;
  try {
    const removed = await invoke("delete_jack", { name: j.name, stamp: j.stamp ?? null });
    selKey = null;
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
    .map((j) => j.label)
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
      failed.push(j.label);
    }
  }
  marked.clear();
  selKey = null;
  await load();
  if (removed.length) offerUndo(removed, failed);
  else alertish(`could not delete ${failed.join(", ")}`);
}

// To the other vault, table and all. One call per device, so a refusal names the one
// it was on and the rest still move.
async function moveVault(js) {
  const to = VAULTS[otherVault(js[0]?.vault)]?.[1];
  const moved = [];
  for (const j of js) {
    try {
      await invoke("move_jack", { name: j.name });
      moved.push(j.label);
    } catch (e) {
      alertish(e);
    }
  }
  marked.clear();
  selKey = null;
  await load();
  if (moved.length) {
    flash(`Moved ${moved.length === 1 ? `"${moved[0]}"` : `${moved.length} devices`} to ${to}.`);
  }
}

async function newGroup(parent) {
  const name = await ask(
    parent?.path ? `New folder inside ${parent.path}` : "New folder",
    "",
    "Create",
  );
  if (!name) return;
  const leaf = name.replace(/^\/+|\/+$/g, "");
  // A new folder is in a vault like any other; the team's when nobody said.
  const vault = parent?.vault ?? (vaults.two ? "team" : undefined);
  const id = { path: parent?.path ? `${parent.path}/${leaf}` : leaf, vault };
  pending.set(gkey(id), id);
  openGroup(gkey({ path: id.path.split("/")[0], vault }));
  pickGroup(id, false);
}

async function renameGroup(id) {
  const to = await ask(`Rename folder ${id.path} to`, id.path.split("/").pop(), "Rename");
  if (!to) return;
  const parent = id.path.includes("/") ? id.path.slice(0, id.path.lastIndexOf("/")) : "";
  const next = { path: parent ? `${parent}/${to}` : to, vault: id.vault };
  try {
    await invoke("rename_group", { from: id.path, to: next.path, vault: id.vault ?? null });
    if (pending.delete(gkey(id))) pending.set(gkey(next), next);
    if (sameGroup(group, id)) group = next;
    await load();
  } catch (e) {
    alertish(e);
  }
}

const underFolder = (id) => (j) =>
  inVault(j, id.vault) && j.folders.some((f) => f === id.path || f.startsWith(id.path + "/"));

async function removeGroup(id) {
  const n = all.filter(underFolder(id)).length;
  const msg = `Remove folder "${id.path}" from ${n} device${n === 1 ? "" : "s"}? The devices stay.`;
  if (!(await ask(msg, null, "Remove"))) return;
  try {
    await invoke("delete_group", { path: id.path, vault: id.vault ?? null });
    pending.delete(gkey(id));
    if (
      group?.vault === id.vault &&
      (group.path === id.path || group.path?.startsWith(id.path + "/"))
    )
      // Up to the vault it was in, not out of it.
      group = vaults.two ? { path: null, vault: id.vault } : null;
    await load();
  } catch (e) {
    alertish(e);
  }
}

// Loops the single-folder command, like `removeMarked`. A folder inside another marked
// one goes with it, so it is neither counted nor asked for twice. Marks are all in one
// vault, so the first one's is everyone's.
async function removeFolders(ids) {
  const vault = ids[0]?.vault;
  const paths = ids.map((id) => id.path);
  const tops = paths.filter((p) => !paths.some((q) => p.startsWith(q + "/")));
  if (tops.length === 1) return removeGroup({ path: tops[0], vault });
  const under = (f) => tops.some((p) => f === p || f.startsWith(p + "/"));
  const n = all.filter((j) => inVault(j, vault) && j.folders.some(under)).length;
  const msg =
    `Remove ${tops.length} folders from ${n} device${n === 1 ? "" : "s"}? The devices stay.` +
    `\n\n${tops.slice(0, 8).join(", ")}${tops.length > 8 ? `, and ${tops.length - 8} more` : ""}`;
  if (!(await ask(msg, null, "Remove"))) return;
  const failed = [];
  for (const p of tops) {
    try {
      await invoke("delete_group", { path: p, vault: vault ?? null });
      pending.delete(gkey({ path: p, vault }));
    } catch {
      failed.push(p);
    }
  }
  markedFolders.clear();
  if (group?.vault === vault && group.path && under(group.path)) {
    group = vaults.two ? { path: null, vault } : null;
  }
  await load();
  if (failed.length) alertish(`could not remove ${failed.join(", ")}`);
}

// Put devices in a folder. From inside a folder it is a move: every entry under
// `from` becomes `to`, and `to` of null takes them out of it. From nowhere in
// particular it is an add, because there is nothing to move out of. Only the folders
// list is written, so a hand-written key on the device survives. `vault` is the one
// the target folder is in: a drag stays inside it, because the other vault is another
// file and putting a device there is "Move to", said out loud.
async function moveJacks(js, to, from = group?.path ?? null, vault = js[0]?.vault ?? undefined) {
  const here = js.filter((j) => inVault(j, vault));
  if (here.length < js.length) {
    alertish(
      `a drag stays inside one vault - "Move to ${VAULTS[vault]?.[1] ?? "Team"}" puts a device in the other`,
    );
  }
  const under = (f) => from !== null && (f === from || f.startsWith(from + "/"));
  let moved = 0;
  for (const j of here) {
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
    openGroup(gkey({ path: to.split("/")[0], vault }));
    pending.delete(gkey({ path: to, vault }));
  }
  marked.clear();
  await load();
}

// "Move to folder…": the same move, typed. The list of folders is offered as you type.
async function moveAsked(js) {
  const what = js.length === 1 ? `"${js[0].label}"` : `${js.length} devices`;
  offerFolders(js[0]?.vault ?? undefined);
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
let impFound = [],
  impRead = null;

// The vault ticked in the sheet; undefined with one vault, as `inVault` takes it.
const impInto = () => (vaults.two ? impForm["imp-vault"].value : undefined);

// `where` names the source in the header: a path for an ssh config, a file name for
// a document.
function openImport(r, where) {
  impErr.hidden = true;
  $("imp-title").textContent = `Import from ${where}`;
  $("imp-find").value = "";
  // The vault being looked at, the team's when neither is.
  $("imp-vault").hidden = !vaults.two;
  impForm["imp-vault"].value = group?.vault ?? "team";
  impWrap.hidden = false;
  impRead = r;
  renderImport();
}

// Picking the other vault redraws, so "already here" means here.
impForm.addEventListener("change", (e) => {
  if (e.target.name !== "imp-vault") return;
  renderImport();
  $("imp-find").dispatchEvent(new Event("input"));
});

function renderImport() {
  const r = impRead;
  // Devices already in the vault this lands in are shown unticked, so running it twice
  // is safe. By the key a file would have, which is what an import writes.
  const into = impInto();
  impFound = r.hosts.map((h) => ({
    ...h,
    here: all.some((j) => inVault(j, into) && bareKey(j.name) === h.name),
  }));
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
    setTimeout(async () => {
      await closeSettings();
      // Not over a sheet that stayed open on a refusal.
      if (setWrap.hidden) openImport(r, r.what);
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
          telnet: h.telnet ?? null,
          url: h.url ?? null,
          // An rdp or web device is not also an ssh host; without this every import
          // opens a shell, because that is what `primary` picks first.
          ssh: h.ssh ?? null,
          os: h.os ?? null,
          desc: h.desc ?? null,
        },
        vault: impInto() ?? null,
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
        ["Previous, next tab or pane", [`${chord("[")}`, `${chord("]")}`]],
        ["Open a device beside the tab in front", [chord("d")]],
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
        ["Mark several devices or folders", [pickChord, "Shift-click"]],
        ["Mark every device in view", [chord("a")]],
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
    [
      "In a remote desktop",
      [
        // ⌘ is the Windows key there, so the far end's own paste is Ctrl.
        ["Paste what this machine copied, files too", ["Ctrl+V"]],
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
  snipRefused = saveRefused = false;
  // Same selector the submit handler writes through; theme and font size are not checkboxes.
  for (const el of setForm.querySelectorAll("input[type=checkbox]")) el.checked = !!prefs[el.name];
  setForm.elements.theme.value = ["light", "dark"].includes(prefs.theme) ? prefs.theme : "system";
  setForm.elements.font_size.value = termFont();
  setForm.elements.ssh_config_vault.value = prefs.ssh_config_vault === "mine" ? "mine" : "team";
  $("ssh-vault-field").hidden = !vaults.two;
  setForm.elements.list.value = prefs.list ?? "";
  showListWhere();
  setForm.elements.bitwarden_server.value = prefs.bitwarden_server ?? "";
  // Only worth saying while there are two files.
  const shared = cfgPath && ownPath && cfgPath !== ownPath;
  $("team-own").hidden = $("cfg-own").hidden = !shared;
  $("team-own").textContent = vaults.copies
    ? `${vaults.copies} of your own ${vaults.copies === 1 ? "is an unchanged copy" : "are unchanged copies"} of the team's.`
    : `Your own devices stay in ${ownPath}.`;
  $("team-copies").hidden = !shared || !vaults.copies;
  $("team-copies").innerHTML = `${icon("trash-2")}Remove the copies`;
  $("cfg-own").textContent = `Settings and colours stay in ${ownPath}, this machine's own.`;
  renderSwatches();
  $("swatchrow").hidden = !setForm.elements.os_colors.checked;
  for (const el of setWrap.querySelectorAll("[data-icon]")) {
    if (!el.firstChild) el.innerHTML = icon(el.dataset.icon);
  }
  resetSources();
  snipDraft = snips.map((s) => ({ ...s }));
  snipWas = JSON.stringify(snipDraft);
  renderSnips();
  showPane(pane);
  $("new-list").innerHTML = `${icon("folder-plus")}New…`;
  $("pick-list").innerHTML = `${icon("folder-open")}Open…`;
  // Hidden rather than disabled where the OS can't host one at all: a pane that
  // could only ever say no is a pane not worth a row in the list.
  invoke("webext_supported")
    .then((ok) => ($("setnav").querySelector('[data-pane="webext"]').hidden = !ok))
    .catch(() => {});
  sayWebext();
  $("page-openconfig").innerHTML = `${icon("file-pen-line")}Open config file`;
  $("page-checkupdate").innerHTML = `${icon("rotate-cw")}Check for updates`;
  $("page-forget").innerHTML = `${icon("trash-2")}Forget all`;
  // When the last check happened is still true; its answer is not.
  $("update-said").textContent = lastChecked ? `Checked at ${lastChecked}` : "Not checked yet.";
  invoke("app_version")
    .then((v) => ($("appversion").textContent = `patchbay ${v}`))
    .catch(() => ($("appversion").textContent = "patchbay"));
  $("cfgpath").textContent = cfgPath;
  $("keylist").innerHTML = keysHtml();
  $("lic-save").innerHTML = `${icon("check")}Add licence`;
  $("lic-remove").innerHTML = `${icon("trash-2")}Remove`;
  loadLicence();
  setWrap.hidden = false;
}
// Snippets are written here rather than as they are typed: a row half typed is not one to
// refuse yet. A refusal keeps the sheet open on it; closing again drops the draft, so a
// row nobody can fix never traps the sheet.
let snipRefused = false;
// The same for a setting that didn't save: closed once it would look on and not be.
let saveRefused = false;
async function closeSettings() {
  await settingsSaved;
  if (!setErr.hidden && !snipRefused && !saveRefused) {
    saveRefused = true;
    setErr.textContent = `${setErr.textContent} - close again to leave it unsaved`;
    return;
  }
  const changed = JSON.stringify(snipDraft) !== snipWas;
  if (changed && !snipRefused) {
    try {
      await saveSnips(snipRows());
      listStamp = null;
      await load();
    } catch (err) {
      snipRefused = true;
      showPane("snippets");
      showErr(setErr, `${err} - close again to discard the change`);
      // Whatever of the snippets did go in is what the next save is compared against.
      await invoke("snippets")
        .then((r) => ({ list: snips, stamps: snipStamps } = r))
        .catch(() => {});
      return;
    }
  }
  snipRefused = saveRefused = false;
  setWrap.hidden = true;
  if (upWaiting) {
    showUpdate(upWaiting);
    upWaiting = null;
  }
}

// A hidden pane is still in the form, so every field is read on each save.
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

// A list path is long and its end is the part that says which list: the field is
// scrolled to the file, and the line under it reads from the drive down.
function showListWhere() {
  const field = setForm.elements.list;
  const path = field.value.trim();
  const where = $("list-where");
  where.hidden = !path;
  if (path) {
    const { name, rest } = driveOf(path);
    where.innerHTML = [name, ...rest]
      .map((p) => `<span>${esc(p)}</span>`)
      .join(icon("chevron-right"));
  }
  // After the sheet is laid out: a field still hidden has no width to scroll.
  requestAnimationFrame(() => (field.scrollLeft = field.scrollWidth));
}
setForm.elements.list.addEventListener("input", showListWhere);

// Settings take effect as they change, the way the OS's own do, so a switch never looks
// on while doing nothing. Saves queue behind one another: two quick toggles must not
// write over each other with stale `prefs`.
let settingsSaved = Promise.resolve();
const applySettings = () => (settingsSaved = settingsSaved.then(saveSettings));
setForm.addEventListener("change", (e) => {
  // Snippets are written on close, colours by their own handler, an import by its card.
  if (e.target.closest("#snip-rows, #swatches, .sources, #lic-add")) return;
  applySettings();
});
// Enter in a field, or Done: what changed is saved, then the sheet closes.
setForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  await applySettings();
  if (setErr.hidden) closeSettings();
});

async function saveSettings() {
  // Over the current prefs: the sidebar width has no field here and must survive a save.
  const next = { ...prefs };
  for (const el of setForm.querySelectorAll("input[type=checkbox]")) next[el.name] = el.checked;
  next.theme = setForm.elements.theme.value;
  next.ssh_config_vault = setForm.elements.ssh_config_vault.value;
  next.font_size = parseFloat(setForm.elements.font_size.value);
  next.list = setForm.elements.list.value.trim() || null;
  next.bitwarden_server = setForm.elements.bitwarden_server.value.trim() || null;
  // Rust clamps it too, silently; this is the half that can say why.
  if (!(next.font_size >= 8 && next.font_size <= 32)) {
    return showErr(setErr, "terminal font size has to be between 8 and 32");
  }
  setErr.hidden = true;
  saveRefused = false;
  // Done, or Enter, after the change already went in: no second write of the same file.
  if (JSON.stringify(next) === JSON.stringify(prefs)) return;

  try {
    const wasProbing = prefs.probe !== false;
    const server = prefs.bitwarden_server ?? null;
    await invoke("save_settings", { next });
    // Host access is granted when the extension loads, so a new server is a reload.
    if (next.webext && next.bitwarden_server !== server) {
      await invoke("webext_stop").catch(() => {});
      invoke("webext_start").catch(alertish);
    }
    prefs = next;
    listStamp = null;
    // A reload: [defaults] changes what every device shows.
    await load();
    // The throttle would otherwise hold the first sweep back by up to 30s.
    if (!wasProbing && next.probe) {
      lastProbe = 0;
      refreshProbes();
    }
  } catch (err) {
    showErr(setErr, String(err));
  }
}

// ── snippets ───────────────────────────────────────────────────────────────
// The sheet's own copy, written back whole per vault when the sheet closes. A row's vault is where
// it is stored, so changing it is the move.
let snipDraft = [];
let snipWas = "[]";
function renderSnips() {
  const pick = (s, i) =>
    vaults.two
      ? `<select data-si="${i}" data-k="vault" aria-label="Who can see it">${Object.keys(VAULTS)
          .map(
            (v) =>
              `<option value="${v}"${s.vault === v ? " selected" : ""}>${VAULTS[v][1]}</option>`,
          )
          .join("")}</select>`
      : "";
  $("snip-rows").innerHTML = snipDraft.length
    ? snipDraft
        .map(
          (s, i) => `<div class="snip-row${vaults.two ? " two" : ""}">
      <input data-si="${i}" data-k="name" spellcheck="false" placeholder="name" aria-label="Name" value="${esc(s.name)}" />
      <input data-si="${i}" data-k="text" class="mono" spellcheck="false" placeholder="what gets typed" aria-label="Text" value="${esc(s.text)}" />
      ${pick(s, i)}
      <button type="button" class="flat" data-sdel="${i}" aria-label="Remove">${icon("x")}</button>
    </div>`,
        )
        .join("")
    : `<p class="page-note">None yet.</p>`;
  $("snip-add").innerHTML = `${icon("plus")}Add a snippet`;
}
// An edit after a refusal is a fix, to be saved rather than discarded on the next close.
$("snip-rows").addEventListener("input", (e) => {
  snipRefused = false;
  const el = e.target.closest("[data-si]");
  if (el) snipDraft[+el.dataset.si][el.dataset.k] = el.value;
});
$("snip-rows").addEventListener("click", (e) => {
  const del = e.target.closest("[data-sdel]");
  if (!del) return;
  snipRefused = false;
  snipDraft.splice(+del.dataset.sdel, 1);
  renderSnips();
});
$("snip-add").addEventListener("click", () => {
  // Yours unless you say otherwise: a line typed onto a colleague's prompt is
  // something to choose to share.
  snipRefused = false;
  snipDraft.push({ name: "", text: "", vault: vaults.two ? "mine" : null });
  renderSnips();
  $("snip-rows").querySelector(".snip-row:last-child input")?.focus();
});

// What the sheet's rows come to, checked before anything in the sheet is written: a
// refusal here must not leave the settings saved and the snippets not.
function snipRows() {
  const rows = snipDraft
    .map((s) => ({ ...s, name: s.name.trim() }))
    .filter((s) => s.name || s.text);
  const bad = rows.find((s) => !s.name || !snippetOk(s.text));
  if (bad) {
    throw bad.name
      ? `"${bad.name}": a snippet is one line of plain text - it can't press Enter`
      : "a snippet needs a name";
  }
  return rows;
}

// Each vault's table whole, and only the ones that changed. The vault that gained a
// row goes first, so a failure in between leaves a snippet in both rather than neither.
// ponytail: rows moved both ways in one save can't both go first; the second write
// failing then loses the ones that left its vault, until they are typed again.
async function saveSnips(rows) {
  if (JSON.stringify(snipDraft) === snipWas) return;
  const of = (list, v) => list.filter((s) => (s.vault ?? null) === v).map((s) => [s.name, s.text]);
  const order = vaults.two ? ["mine", "team"] : [null];
  const gained = (v) => of(rows, v).length - of(snips, v).length;
  for (const v of [...order].sort((a, b) => gained(b) - gained(a))) {
    const next = of(rows, v);
    if (JSON.stringify(next) === JSON.stringify(of(snips, v))) continue;
    await invoke("save_snippets", { vault: v, list: next, stamp: snipStamps[v ?? "team"] ?? null });
  }
}
$("team-copies").addEventListener("click", async () => {
  try {
    const n = await invoke("remove_copies");
    await load();
    flash(`Removed ${n} cop${n === 1 ? "y" : "ies"} from Mine.`);
    $("team-own").textContent = `Your own devices stay in ${ownPath}.`;
    $("team-copies").hidden = true;
  } catch (e) {
    showErr(setErr, String(e));
  }
});
// What Bitwarden for Mac has installed. Read rather than assumed, so an extension
// WKWebExtension won't take says so here instead of leaving a toggle that does nothing.
async function sayWebext() {
  const said = $("webext-said");
  said.textContent = "Reading…";
  try {
    // Started when the toggle is on, so the answer describes what is *running*
    // rather than what is merely on disk. A second start just reports.
    const on = setForm.elements.webext.checked;
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
// The chips only mean anything while colouring is on.
setForm.elements.os_colors.addEventListener("change", (e) => {
  $("swatchrow").hidden = !e.target.checked;
});

// The picker answers with null when it is cancelled, which must not clear a path
// somebody typed.
async function pickList(folder) {
  try {
    const path = await invoke("pick_list_file", { folder });
    if (path) setForm.elements.list.value = path;
    showListWhere();
    // Set from here, so no `change` fires to save it.
    if (path) applySettings();
  } catch (err) {
    alertish(err);
  }
}
$("new-list").addEventListener("click", () => pickList(true));
$("pick-list").addEventListener("click", () => pickList(false));
$("page-openconfig").addEventListener("click", () => invoke("open_config").catch(alertish));
$("page-checkupdate").addEventListener("click", () => checkUpdates($("update-said")));
$("page-forget").addEventListener("click", () => forgetRdp());

// ── licence ────────────────────────────────────────────────────────────────
// Rust decides (licence.rs); this only says what it decided and passes a paste in.
// `s` is null with one vault: one person needs no licence.
function sayLicence(s, said = "") {
  $("lic-state").textContent = !s
    ? "Free for you"
    : {
        unlicensed: vaults.on_sale ? "No licence for the team's list" : "No licence needed yet",
        licensed: `Licensed to ${s.company}${s.until ? `, until ${licDay(s.until)}` : ""}`,
        grace: `${s.company}'s licence ended on ${licDay(s.until ?? 0)}`,
        ended: `${s.company}'s licence ended on ${licDay(s.until ?? 0)}`,
      }[s.state];
  $("lic-said").textContent =
    said ||
    (s?.state === "grace"
      ? `The team's list keeps opening until ${licDay(s.closes)}.`
      : !s
        ? "Only a team on a shared list needs one."
        : "");
  $("lic-add").hidden = !s;
  $("lic-remove").hidden = !s || s.state === "unlicensed";
  $("lic-free").hidden = vaults.on_sale;
}
async function loadLicence() {
  const s = await invoke("licence").catch(() => null);
  sayLicence(s);
  return s;
}
// The button waits for something to take; Enter in the box is a newline, not a save.
$("lic-text").addEventListener("input", () => {
  $("lic-save").disabled = !$("lic-text").value.trim();
});
$("lic-save").addEventListener("click", async () => {
  const text = $("lic-text").value;
  if (!text.trim()) return $("lic-text").focus();
  $("lic-save").disabled = true;
  try {
    const s = await invoke("save_licence", { text });
    $("lic-text").value = "";
    $("lic-save").disabled = true;
    sayLicence(s, "Added beside the shared list, for everyone on it. Thank you.");
    listStamp = null;
    await load();
  } catch (e) {
    $("lic-said").textContent = String(e);
  } finally {
    $("lic-save").disabled = !$("lic-text").value.trim();
  }
});
$("lic-remove").addEventListener("click", async () => {
  if (!(await ask("Remove the licence for everyone on the shared list?", null, "Remove"))) return;
  try {
    await invoke("remove_licence");
    sayLicence(await invoke("licence"), "Removed.");
    listStamp = null;
    await load();
  } catch (e) {
    $("lic-said").textContent = String(e);
  }
});
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
  offerFolders(vaults.two ? jackForm.elements.vault.value : undefined);
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
for (const r of jackForm.elements.vault) r.addEventListener("change", renderFolderSuggestions);
$("crumbs").addEventListener("click", (e) => {
  const dropped = e.target.closest("[data-drop]")?.dataset.drop;
  if (!dropped) return;
  jackForm.elements.folders.value = enteredFolders()
    .filter((f) => f !== dropped)
    .join(", ");
  renderCrumbs();
});

// No name is every device in the list.
async function forgetRdp(name = null) {
  for (const typed of [rdpCreds, vncPasswords]) {
    if (name) typed.delete(name);
    else typed.clear();
  }
  try {
    await invoke("rdp_forget", { name });
    flash(name ? `Forgot the password for ${name}` : "Forgot every saved password");
  } catch (e) {
    alertish(e);
  }
}
