// Shared state and the helpers every other ui/ script leans on.
// Loaded first: nothing here may reference the later files at top level.
// Separate file, not an inline <script>, so the CSP can stay `script-src 'self'`.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const appEl = $("app");
const treeEl = $("tree"), listEl = $("list"), detailEl = $("d-body");
const detailPane = $("detail"), dActions = $("d-actions");
const searchBtn = $("searchbtn");
const paletteEl = $("palette"), pq = $("pq"), presultsEl = $("presults");
const ctxEl = $("ctx");
const sheetWrap = $("sheetwrap"), jackForm = $("jackform"), jfErr = $("jf-err"), jfDelete = $("jf-delete");
const askWrap = $("askwrap"), askForm = $("askform"), askInput = $("ask-input"), askErr = $("ask-err");
const askUserField = $("ask-user-field"), askUser = $("ask-user"), askLabel = $("ask-label");
const askBody = askForm.querySelector(".sheet-body");
const setWrap = $("setwrap"), setForm = $("setform"), setErr = $("set-err");
const impWrap = $("importwrap"), impForm = $("importform"), impList = $("imp-list");
const impNote = $("imp-note"), impErr = $("imp-err"), impOk = $("imp-ok");
const setNav = $("setnav");
const upWrap = $("uptoast"), upText = $("up-text"), upInstall = $("up-install"), upClose = $("up-close");
// When the last check answered, as a clock time. The launch check says nothing when
// there is nothing to install, so without this the settings pane cannot tell "current"
// from "never asked" - which are the same sentence to read and not the same thing.
let lastChecked = null;
const checkedNow = () => (lastChecked = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
const upMore = $("up-more"), upNotes = $("up-notes");
const msgWrap = $("msg"), msgText = $("msg-text"), msgClose = $("msg-close");

const isMac = navigator.userAgent.includes("Mac");
if (isMac) document.body.dataset.os = "macos";
// Shortcut labels: ⌘K on macOS, Ctrl+K everywhere else. The handler already accepts both.
const chord = (k) => (isMac ? `⌘${k.toUpperCase()}` : `Ctrl+${k.toUpperCase()}`);

// A folder is keyed by its path, and `null` is the row above all of them. Sets and
// Maps only - the path travels to Rust on its own, never as this.
const gkey = (g) => (g ? (g.path ?? "") : "");
const sameGroup = (a, b) => gkey(a) === gkey(b);

let all = [];                 // every jack, in file order
let shown = [];               // what the middle column currently lists
let probes = new Map();       // name -> { target, ms }
// The selected row: { path }. `group` itself null is every device.
let group = null;
let expanded = new Set();     // open rows, by gkey
let sel = 0;
// Rows picked out for an action about several devices at once, by name rather than by
// index: the list is refiltered under them by every render, and an index would then
// point at whichever device had moved into that row.
let marked = new Set();
let palSel = 0;
let editing = null;   // jack name being edited, or null when adding
// Folders only exist because a jack carries the tag, so a brand-new empty one is
// held here until something lands in it. Dropped on reload, which is honest.
let pending = new Map();      // gkey -> { path }
let notes = new Map();        // folder path -> the note hung on it
let seeded = false;           // the tree's initial expansion is a one-off
let lastProbe = 0;            // epoch ms of the last sweep, for the throttle below
let detailMode = "jack";      // what the right pane describes: "jack" or "group"
// How the middle column lists: "list" is the flat one, "map" groups by the route to
// each device. The sidebar tree is how *you* filed them; the map is the shape of the
// network, which nothing else in the app can draw because nothing else resolves a chain.
let listMode = "list";
let prefs = {};               // [settings] from the config
let colors = {};              // [colors] overrides, os key -> hex
let cfgPath = "";             // where the config lives, shown on first run
let sshKeys = [];             // private keys found in ~/.ssh, to suggest in the key field
let tunnels = [];             // live ssh -L forwards holding RDP open

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

// A sheet is modal to the keyboard; the palette is not, it does its own. Kept apart
// because folding them together would let a ⌘K opened over a sheet change which one
// Escape closes.
const sheetOpen = () => [sheetWrap, askWrap, setWrap, impWrap].some((el) => !el.hidden);
// Anything painting over the window at all - palette and context menu included. A web
// tab is an OS view stacked above the page, so it has to shrink away for every one of
// these. The sidebar stays live while a session tab is open, so a right-click menu
// lands over the webview and is otherwise half-covered by it.
const OVERLAYS = () => [sheetWrap, askWrap, setWrap, impWrap, paletteEl, ctxEl];
const modalOpen = () => OVERLAYS().some((el) => !el.hidden);

// Lucide, inlined at generate time by scripts/icons.mjs - see ui/icons.js.
const icon = (name) =>
  `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
     stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] ?? ""}</svg>`;

// `os = "..."` in the config, matched loosely so "Ubuntu 22.04" and "ubuntu" agree.
// Brand marks are fill paths (simple-icons); the fallbacks are stroked Lucide shapes.
const osKey = (os) => (os ?? "").toLowerCase().replace(/[^a-z]/g, "");
function osIcon(os) {
  const k = osKey(os);
  const brand = Object.keys(BRANDS).find((b) => k.startsWith(b) || (b.startsWith(k) && k.length > 2));
  if (k && brand) {
    return `<svg class="i brand" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="${BRANDS[brand]}"/></svg>`;
  }
  const fb = Object.keys(BRAND_FALLBACKS).find((b) => k === b);
  return icon(fb ? BRAND_FALLBACKS[fb] : "server");
}
// ── colour ─────────────────────────────────────────────────────────────────
// Brand hexes are chosen for print, not for our panel: macOS is pure black,
// Linux is near-white yellow, nine of them fail contrast on one theme or other.
// So nudge lightness until the colour is readable, rather than shipping defaults
// that look broken and expecting people to fix them by hand.
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
    return Math.round(from + (to - from) * t).toString(16).padStart(2, "0");
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

// Comes out of a file people hand-edit, and reaches xterm's layout, so it is clamped
// again here rather than only on the way in.
const termFont = () => Math.min(32, Math.max(8, +prefs.font_size || 12.5));

let painted = "";   // the theme and font last drawn, so an alt-tab is not a repaint

/// The theme is an attribute rather than a media query, so a pinned one survives the
/// machine's own setting. `localStorage` is only how theme.js paints the right frame
/// before [settings] has been read - `prefs` is what decides, and the window resolves
/// "system", because the page's own `color-scheme` poisons `prefers-color-scheme`.
async function applyTheme() {
  const want = prefs.theme === "light" || prefs.theme === "dark" ? prefs.theme : "system";
  const now = document.documentElement.dataset.theme;
  const pick = await invoke("set_theme", { theme: want })
    .catch(() => (want === "system" ? now || "dark" : want));
  document.documentElement.dataset.theme = pick;
  try { localStorage.theme = pick; } catch { /* nothing to remember with */ }
  // load() runs on every window focus, and assigning xterm's theme repaints whether or
  // not it changed. The render is the other half: `readable()` nudges a brand colour
  // against the surface, so every mark on screen was computed for the old one.
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
  const brand = Object.keys(BRAND_COLORS).find((b) => k.startsWith(b) || (b.startsWith(k) && k.length > 2));
  return brand ? readable(BRAND_COLORS[brand]) : null;
}

const hit = (j, f) =>
  !f || j.name.toLowerCase().includes(f) || j.host.toLowerCase().includes(f) ||
  (j.desc ?? "").toLowerCase().includes(f) || j.folders.some((x) => x.toLowerCase().includes(f));

// The names you have actually opened, most recent first, so the palette answers with
// the box you were on ninety seconds ago rather than with whatever the file lists
// first. `localStorage` and not the config: this is a habit rather than part of the
// list, it changes several times an hour, and the config is a file people hand-edit.
// A name that no longer exists just never matches - there is nothing to prune.
let recent = [];
try { recent = JSON.parse(localStorage.recent ?? "[]"); } catch { /* nothing to remember with */ }
if (!Array.isArray(recent)) recent = [];
// The tabs that were open when the window last closed, reopened on the next launch.
// Read here, before the first renderTabs writes today's empty strip over it.
let lastTabs = [];
try { lastTabs = JSON.parse(localStorage.tabs ?? "[]"); } catch { /* nothing to remember with */ }
if (!Array.isArray(lastTabs)) lastTabs = [];

function used(name) {
  recent = [name, ...recent.filter((n) => n !== name)].slice(0, 40);
  try { localStorage.recent = JSON.stringify(recent); } catch { /* nothing to remember with */ }
}
