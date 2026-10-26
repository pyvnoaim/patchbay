// Shared state and the helpers every other ui/ script uses. Loaded first: classic
// scripts share one global scope, so nothing here may reference a later file at top level.
//
// A selected folder is `{ path, vault }`, not a bare string. Everything keyed by a folder -
// `expanded`, `pending`, the notes, the folder marks - goes through `gkey()`, so there is one shape
// for "which folder" and one place to change it. A vault's own row is `{ path: null, vault }`. A
// drag writes `folders` and stays inside a vault; the other vault is another file, and getting
// there is "Move to Team/Mine".
//
// Only the palette ranks by recency. Every open (`connect`, web, RDP, VNC, files) calls
// `used(name)`, which keeps the last forty names in `localStorage` - a habit, not part of the list,
// so it never touches the config. Folders are A to Z and the devices in one are in file order,
// never by recency: rows moving under someone reading them is a worse list, not a smarter one. The
// theme, the tabs to reopen, and the sidebar's open folders (`keepExpanded()`, read back once per
// launch, so a folder folded stays folded and a fresh machine starts with all of them closed) are
// there for the same reason. Those four are everything `localStorage` holds, and every one of them
// is a habit this machine has rather than something the list says.
//
// Anything that goes wrong outside a form goes through `alertish()`, which flashes a pill. It used
// to write into the detail pane's command box, which isn't there when a folder is selected and is
// gone entirely under 720px - so a failed delete could say nothing at all. `#notices` holds two
// pills, not one: an error arriving mid-download must not take the update's own Restart button off
// the screen.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const appEl = $("app");
const treeEl = $("tree"),
  listEl = $("list"),
  detailEl = $("d-body");
const detailPane = $("detail");
const searchBtn = $("searchbtn");
const paletteEl = $("palette"),
  pq = $("pq"),
  presultsEl = $("presults");
const ctxEl = $("ctx");
const sheetWrap = $("sheetwrap"),
  jackForm = $("jackform"),
  jfErr = $("jf-err"),
  jfDelete = $("jf-delete");
const askWrap = $("askwrap"),
  askForm = $("askform"),
  askInput = $("ask-input"),
  askErr = $("ask-err");
const askUserField = $("ask-user-field"),
  askUser = $("ask-user"),
  askLabel = $("ask-label");
const askBody = askForm.querySelector(".sheet-body");
const setWrap = $("setwrap"),
  setForm = $("setform"),
  setErr = $("set-err");
const impWrap = $("importwrap"),
  impForm = $("importform"),
  impList = $("imp-list");
const impNote = $("imp-note"),
  impErr = $("imp-err"),
  impOk = $("imp-ok");
const setNav = $("setnav");
const upWrap = $("uptoast"),
  upText = $("up-text"),
  upInstall = $("up-install"),
  upClose = $("up-close");
// When the last update check answered, so the settings pane can tell "current" from
// "never asked".
let lastChecked = null;
const checkedNow = () =>
  (lastChecked = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
const upMore = $("up-more"),
  upNotes = $("up-notes");

const isMac = navigator.userAgent.includes("Mac");
if (isMac) document.body.dataset.os = "macos";
// Shortcut labels: ⌘K on macOS, Ctrl+Shift+K elsewhere. Shift because a session owns
// plain Ctrl: Ctrl+W is a shell's delete-word and Ctrl+[ is its Escape.
const chord = (k) => (isMac ? `⌘${k.toUpperCase()}` : `Ctrl+Shift+${k.toUpperCase()}`);
/** Whether a keydown carries the window's modifier, and which key it names: Shift
 *  changes what `key` says, so punctuation is read off the physical key. */
const chorded = (e) => (isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && e.shiftKey);
const chordKey = (e) =>
  ({ BracketLeft: "[", BracketRight: "]", Comma: ",", Slash: "/" })[e.code] ?? e.key.toLowerCase();
// Picking rows out is the OS's click gesture, not the window's chord: plain Ctrl off
// macOS, because Shift-click is already the range.
const picking = (e) => (isMac ? e.metaKey : e.ctrlKey);
const pickChord = isMac ? "⌘-click" : "Ctrl-click";

// The key for a folder in Sets and Maps; null is every device. A folder is in one
// vault, so with two the vault is part of which folder it is. Rust gets the path and
// the vault's word, never this.
const gkey = (g) => (g ? `${g.vault ? `${g.vault}:` : ""}${g.path ?? ""}` : "");
const sameGroup = (a, b) => gkey(a) === gkey(b);

let all = []; // every jack, in file order
let shown = []; // the device rows the tree is drawing, top to bottom
let shownIn = []; // the folder each of those rows sits in, as a { path, vault }
// The selected row by what it is, so it survives a folder unfolding above it.
let selKey = null;
// Two once a shared list is set: yours and the team's. `away` is why the team's is missing.
let vaults = { two: false, team: null, away: null, copies: 0 };
let detailsOpen = false; // the side pane beside a session, behind the Details chip
// A route walked one link at a time, by device: { busy } while asking, then { steps }.
// Held for the run, never kept: it was true when it was asked.
let breaks = new Map();
// What this machine and the team each know of a team device's host key, by device.
// `null` while the answer is on its way, so it is asked for once.
let hostKeys = new Map();
// Who else has a session on a team device right now: name -> [who]. Empty without a
// shared list.
let present = new Map();
let presentSaid = ""; // what was last told about this window, so a redraw isn't a write
// Lines of text the palette types at a prompt, from both vaults, yours first.
let snips = []; // { name, text, vault }
let snipStamps = {}; // each vault's [snippet] table as read, for the save
let snipsRefused = ""; // the refused names already said, so a focus doesn't repeat them
// One plain line, at most 1000 characters. Rust refuses the same on save and on read;
// this is the check on the way out, the last thing between a file and a prompt.
const snippetOk = (t) =>
  t.trim().length > 0 && [...t].length <= 1000 && !/[\u0000-\u001f\u007f-\u009f]/.test(t);
let probes = new Map(); // name -> { target, ms }
// The selected folder: { path, vault }, or null for every device.
let group = null;
// Where the list starts, and where Escape goes back to: your own devices with a shared
// list set, every device with one list.
const homeGroup = () => (vaults.two ? { path: null, vault: "mine" } : null);
let expanded = new Set(); // open rows, by gkey
let sel = 0;
// Marks are names, not indexes: the list is refiltered under them by every render.
let marked = new Set();
// Folders marked in the tree, gkey -> id, all in one vault. Never alongside `marked`:
// one dock, one Delete.
let markedFolders = new Map();
let palSel = 0;
let editing = null; // jack name being edited, or null when adding
let editStamp = null; // the table as it was when the sheet opened, sent back with the save
// A new empty folder, held until a device lands in it. Dropped on reload.
let pending = new Map(); // gkey -> { path }
let notes = new Map(); // folder path -> the note hung on it
let noteStamps = new Map(); // folder path -> the folder's table as read, for the save
let folderDefs = new Map(); // folder path -> the user, port, key and jump it lends its devices
let seeded = false; // the tree's initial expansion is a one-off
let lastProbe = 0; // epoch ms of the last sweep, for the throttle below
// A press in flight, so a render doesn't replace the button under it. WebKit fires
// the click on the nearest ancestor mousedown and mouseup still share, so a pane
// rebuilt in between turns Ping into nothing at all - which is why a button needed
// pressing twice after clicking back into the window: `focus` reloads and redraws.
let pressing = false;
let missedRender = false;
let webextRunning = false; // Bitwarden is loaded, so web tabs get its key button
let detailMode = "group"; // what the detail describes: "jack" or "group"
// "map" draws the selected folder by the route to each device, in place of the detail.
let listMode = "list";
let prefs = {}; // [settings] from the config
let colors = {}; // [colors] overrides, os key -> hex
let cfgPath = ""; // where the list lives, shown on first run
let ownPath = ""; // this machine's config; the same file unless a team list is set
let loadErr = ""; // the last failure load() flashed, so a focus doesn't repeat it
let listStamp = null; // mtime and size of the list, as the poll last saw them
let lastConflict = ""; // the conflicted copy already named, so the pill shows once
let sshKeys = []; // private keys found in ~/.ssh, to suggest in the key field
let tunnels = []; // live ssh -L forwards holding RDP open

// A private window, or a webview with site data blocked, throws on the accessor itself.
// Everything kept here is a habit rather than part of the list, so losing it costs nothing.
const remember = (key, value) => {
  try {
    localStorage[key] = value;
  } catch {
    /* nothing to remember with */
  }
};
const recallList = (key) => {
  try {
    const had = JSON.parse(localStorage[key] ?? "[]");
    return Array.isArray(had) ? had : [];
  } catch {
    return [];
  }
};

// Which folders are open is this machine's habit, not part of the list, so it lives
// beside the theme and the palette's recency rather than in the config.
const keepExpanded = () => remember("expanded", JSON.stringify([...expanded]));
const openGroup = (key) => {
  expanded.add(key);
  keepExpanded();
};
const toggleGroup = (key) => {
  expanded.has(key) ? expanded.delete(key) : expanded.add(key);
  keepExpanded();
};

const esc = (s) =>
  String(s ?? "").replace(
    /[&<>"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c],
  );

// A self-dismissing line at the foot of the window for anything outside a form. Each is its
// own pill, stacked, so a second message can't take the first one's Undo off the screen.
// `action` is `{ label, run }`: a button on the pill, gone with it. Returns the pill.
// ponytail: four at once, the oldest goes; a count badge if something ever fires dozens.
function flash(text, bad = false, action = null) {
  const el = document.createElement("div");
  el.className = bad ? "pill msg bad" : "pill msg";
  el.setAttribute("role", "alert");
  el.innerHTML =
    `<div class="pill-row"><span class="msg-text"></span>` +
    (action ? `<button type="button" class="primary">${esc(action.label)}</button>` : "") +
    `<button type="button" class="x" aria-label="Dismiss">${icon("x")}</button></div>`;
  el.querySelector(".msg-text").textContent = text;
  let fade;
  const gone = () => {
    clearTimeout(fade);
    el.remove();
  };
  el.querySelector(".x").addEventListener("click", gone);
  el.querySelector(".primary")?.addEventListener("click", () => {
    gone();
    action.run();
  });
  fade = setTimeout(
    () => {
      el.classList.add("leaving");
      fade = setTimeout(gone, 280);
    },
    bad || action ? 8000 : 4000,
  );
  // Past four, the oldest plain line goes before any pill still holding an Undo.
  const shown = [...document.querySelectorAll("#notices .msg")];
  if (shown.length >= 4) (shown.find((p) => !p.querySelector(".primary")) ?? shown[0]).remove();
  $("notices").insertBefore(el, $("ssh-leftover"));
  return el;
}

// Every failure the window can't put in a form. A pill, because the detail pane's
// command box is absent when a folder is selected and gone under 960px.
function alertish(e) {
  flash(String(e), true);
}

// A licence's date as the day it names: in UTC, the zone `npm run licence` signs in.
const licDay = (secs) =>
  new Date(secs * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });

// An icon-only control is named by its tooltip, so a screen reader says "Settings"
// rather than "button". Only where the control has no words of its own, and only a name
// set here is kept in step with a tooltip that changes (the URL scheme's does).
function nameIcons(root) {
  const els = root.matches?.("[data-tip]") ? [root] : [];
  els.push(...(root.querySelectorAll?.("[data-tip]") ?? []));
  for (const el of els) {
    if (el.hasAttribute("aria-label") && el.dataset.named === undefined) continue;
    if (el.textContent.trim() && el.dataset.named === undefined) continue;
    el.setAttribute("aria-label", el.dataset.tip.trim());
    el.dataset.named = "";
  }
}

// A sheet is modal to the keyboard; the palette handles its own keys.
const sheetOpen = () => [sheetWrap, askWrap, setWrap, impWrap].some((el) => !el.hidden);
// A tick on a Force Touch trackpad, for a drag that lines up with something: `haptic` in
// commands/app.rs. Nothing off macOS, and nothing to report if it doesn't happen.
const tick = () => invoke("haptic").catch(() => {});

// Everything that paints over the window. A web tab is an OS view above the page and
// shrinks away for each of these; one left out gets the webview painted over it.
const OVERLAYS = () => [sheetWrap, askWrap, setWrap, impWrap, paletteEl, ctxEl];
const modalOpen = () => OVERLAYS().some((el) => !el.hidden);

// Lucide, inlined into ui/gen/icons.js by scripts/icons.mjs.
const icon = (name) =>
  `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
     stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] ?? ""}</svg>`;

// Matched loosely so "Ubuntu 22.04" and "ubuntu" agree. Brand marks are filled paths;
// the fallbacks are stroked Lucide shapes.
const osKey = (os) => (os ?? "").toLowerCase().replace(/[^a-z]/g, "");
const osBrand = (k) =>
  k
    ? Object.keys(BRANDS).find((b) => k.startsWith(b) || (b.startsWith(k) && k.length > 2))
    : undefined;
function osIcon(os) {
  const k = osKey(os);
  const brand = osBrand(k);
  if (brand) {
    return `<svg class="i brand" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="${BRANDS[brand]}"/></svg>`;
  }
  const fb = Object.keys(BRAND_FALLBACKS).find((b) => k.startsWith(b));
  return icon(fb ? BRAND_FALLBACKS[fb] : "server");
}
// ── colour ─────────────────────────────────────────────────────────────────
// Brand hexes are chosen for print; nine fail contrast on one theme. Nudge lightness
// until readable rather than painting them raw.
const srgb = (v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
const lum = (hex) => {
  const [r, g, b] = [1, 3, 5].map((i) => srgb(parseInt(hex.substr(i, 2), 16) / 255));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};
const mix = (hex, target, t) => {
  const ch = (i) => {
    const from = parseInt(hex.substr(i, 2), 16);
    const to = parseInt(target.substr(i, 2), 16);
    return Math.round(from + (to - from) * t)
      .toString(16)
      .padStart(2, "0");
  };
  return `#${ch(1)}${ch(3)}${ch(5)}`;
};

const onDark = () => !matchMedia("(prefers-color-scheme: light)").matches;

/** Nudge a colour toward the readable side until it clears 3:1 on this theme. */
function readable(hex) {
  if (!/^#[0-9a-f]{6}$/i.test(hex)) return hex;
  const dark = onDark();
  // What a row actually sits on: --panel dark, --bg light. Hex, because contrast() needs one.
  const surface = dark ? "#1c1c20" : "#f4f4f6";
  const toward = dark ? "#ffffff" : "#000000";
  let out = hex;
  for (let t = 0; t <= 0.9; t += 0.1) {
    out = mix(hex, toward, t);
    if (contrast(out, surface) >= 3) break;
  }
  return out;
}

// From a hand-edited file and reaches xterm's layout, so clamped here as well.
const termFont = () => Math.min(32, Math.max(8, +prefs.font_size || 12.5));

let painted = ""; // theme and font last drawn; a focus with no change is no repaint

// The theme is an attribute, so a pinned one survives the OS setting. `prefs` decides;
// localStorage only lets theme.js paint the first frame. Rust resolves "system", because
// the page's own `color-scheme` poisons `prefers-color-scheme`.
async function applyTheme() {
  const want = prefs.theme === "light" || prefs.theme === "dark" ? prefs.theme : "system";
  const now = document.documentElement.dataset.theme;
  const pick = await invoke("set_theme", { theme: want }).catch(() =>
    want === "system" ? now || "dark" : want,
  );
  document.documentElement.dataset.theme = pick;
  remember("theme", pick);
  // load() runs on every change to the list; skip the repaint when nothing changed. render() is part
  // of it: readable() computed every colour against the old surface.
  const stamp = `${pick} ${termFont()}`;
  if (stamp === painted) return;
  painted = stamp;
  restyleTerminals();
  render();
}

/** A device's colour: your override first, then the brand's own hex. */
function osColor(os) {
  if (prefs.os_colors === false) return null;
  const k = osKey(os);
  if (!k) return null;
  const own = colors[k] ?? colors[Object.keys(colors).find((c) => k.startsWith(c)) ?? ""];
  if (own) return /^#[0-9a-f]{6}$/i.test(own) ? readable(own) : null;
  const brand = Object.keys(BRAND_COLORS).find(
    (b) => k.startsWith(b) || (b.startsWith(k) && k.length > 2),
  );
  return brand ? readable(BRAND_COLORS[brand]) : null;
}

// `name` is the key everything goes by; `label` is what the window says, and two
// devices in different folders can share one.
const labelOf = (name) => all.find((j) => j.name === name)?.label ?? name;
// A device of your own is `me:<key>` once there are two vaults; a file only has the key.
const bareKey = (name) => name.replace(/^me:/, "");
// The two vaults as the window names them: an icon and a word.
const VAULTS = { mine: ["user", "Mine"], team: ["users", "Team"] };
const vaultIds = () => (vaults.two ? ["mine", "team"] : [undefined]);
const inVault = (j, v) => (j.vault ?? undefined) === v;
const otherVault = (v) => (v === "mine" ? "team" : "mine");

// The url as well as the host: a device that is only a web UI has no host to find it
// by, and an address pasted from a ticket is as likely to be one as the other.
const hit = (j, f) =>
  !f ||
  j.label.toLowerCase().includes(f) ||
  j.host.toLowerCase().includes(f) ||
  (j.url ?? "").toLowerCase().includes(f) ||
  (j.desc ?? "").toLowerCase().includes(f) ||
  j.folders.some((x) => x.toLowerCase().includes(f));

// Names opened, most recent first, for the palette's ranking. localStorage, not the
// config: a habit, not part of the list. A name that no longer exists never matches.
let recent = recallList("recent");
// Tabs open at last close, reopened on launch. Read before the first renderTabs
// overwrites it.
let lastTabs = recallList("tabs");

function used(name) {
  recent = [name, ...recent.filter((n) => n !== name)].slice(0, 40);
  remember("recent", JSON.stringify(recent));
}
