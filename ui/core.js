// Shared state and the helpers every other ui/ script uses. Loaded first: classic
// scripts share one global scope, so nothing here may reference a later file at top level.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const appEl = $("app");
const treeEl = $("tree"),
  listEl = $("list"),
  detailEl = $("d-body");
const detailPane = $("detail"),
  dActions = $("d-actions");
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
const msgWrap = $("msg"),
  msgText = $("msg-text"),
  msgAct = $("msg-act"),
  msgClose = $("msg-close");

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

// The key for a folder in Sets and Maps; null is the row above all of them. Rust gets
// the path itself, never this.
const gkey = (g) => (g ? (g.path ?? "") : "");
const sameGroup = (a, b) => gkey(a) === gkey(b);

let all = []; // every jack, in file order
let shown = []; // what the middle column currently lists
let probes = new Map(); // name -> { target, ms }
// The selected row: { path }, or null for every device.
let group = null;
let expanded = new Set(); // open rows, by gkey
let sel = 0;
// Marks are names, not indexes: the list is refiltered under them by every render.
let marked = new Set();
let palSel = 0;
let editing = null; // jack name being edited, or null when adding
// A new empty folder, held until a device lands in it. Dropped on reload.
let pending = new Map(); // gkey -> { path }
let notes = new Map(); // folder path -> the note hung on it
let seeded = false; // the tree's initial expansion is a one-off
let lastProbe = 0; // epoch ms of the last sweep, for the throttle below
let detailMode = "jack"; // what the right pane describes: "jack" or "group"
// "list" is the flat column; "map" groups devices by the route to them.
let listMode = "list";
let prefs = {}; // [settings] from the config
let colors = {}; // [colors] overrides, os key -> hex
let cfgPath = ""; // where the config lives, shown on first run
let sshKeys = []; // private keys found in ~/.ssh, to suggest in the key field
let tunnels = []; // live ssh -L forwards holding RDP open

const esc = (s) =>
  String(s ?? "").replace(
    /[&<>"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c],
  );

// A sheet is modal to the keyboard; the palette handles its own keys.
const sheetOpen = () => [sheetWrap, askWrap, setWrap, impWrap].some((el) => !el.hidden);
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
function osIcon(os) {
  const k = osKey(os);
  const brand = Object.keys(BRANDS).find(
    (b) => k.startsWith(b) || (b.startsWith(k) && k.length > 2),
  );
  if (k && brand) {
    return `<svg class="i brand" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="${BRANDS[brand]}"/></svg>`;
  }
  const fb = Object.keys(BRAND_FALLBACKS).find((b) => k === b);
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
  try {
    localStorage.theme = pick;
  } catch {
    /* nothing to remember with */
  }
  // load() runs on every focus; skip the repaint when nothing changed. render() is part
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

const hit = (j, f) =>
  !f ||
  j.name.toLowerCase().includes(f) ||
  j.host.toLowerCase().includes(f) ||
  (j.desc ?? "").toLowerCase().includes(f) ||
  j.folders.some((x) => x.toLowerCase().includes(f));

// Names opened, most recent first, for the palette's ranking. localStorage, not the
// config: a habit, not part of the list. A name that no longer exists never matches.
let recent = [];
try {
  recent = JSON.parse(localStorage.recent ?? "[]");
} catch {
  /* nothing to remember with */
}
if (!Array.isArray(recent)) recent = [];
// Tabs open at last close, reopened on launch. Read before the first renderTabs
// overwrites it.
let lastTabs = [];
try {
  lastTabs = JSON.parse(localStorage.tabs ?? "[]");
} catch {
  /* nothing to remember with */
}
if (!Array.isArray(lastTabs)) lastTabs = [];

function used(name) {
  recent = [name, ...recent.filter((n) => n !== name)].slice(0, 40);
  try {
    localStorage.recent = JSON.stringify(recent);
  } catch {
    /* nothing to remember with */
  }
}
