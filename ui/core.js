// Shared state and the helpers every other ui/ script leans on.
// Loaded first: nothing here may reference the later files at top level.
// Separate file, not an inline <script>, so the CSP can stay `script-src 'self'`.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const treeEl = $("tree"), listEl = $("list"), detailEl = $("d-body");
const detailPane = $("detail"), dActions = $("d-actions");
const searchBtn = $("searchbtn");
const paletteEl = $("palette"), pq = $("pq"), presultsEl = $("presults");
const ctxEl = $("ctx");
const sheetWrap = $("sheetwrap"), jackForm = $("jackform"), jfErr = $("jf-err"), jfDelete = $("jf-delete");
const askWrap = $("askwrap"), askForm = $("askform"), askInput = $("ask-input"), askErr = $("ask-err");
const askUserField = $("ask-user-field"), askUser = $("ask-user"), askLabel = $("ask-label");
const askBody = askForm.querySelector(".sheet-body");
const vpnWrap = $("vpnwrap"), vpnForm = $("vpnform"), vpnErr = $("vpn-err"), vpnDelete = $("vpn-delete");
const setWrap = $("setwrap"), setForm = $("setform"), setErr = $("set-err");
const impWrap = $("importwrap"), impForm = $("importform"), impList = $("imp-list");
const impNote = $("imp-note"), impErr = $("imp-err"), impOk = $("imp-ok");

const isMac = navigator.userAgent.includes("Mac");
if (isMac) document.body.dataset.os = "macos";
// Shortcut labels: ⌘K on macOS, Ctrl+K everywhere else. The handler already accepts both.
const chord = (k) => (isMac ? `⌘${k.toUpperCase()}` : `Ctrl+${k.toUpperCase()}`);

let all = [];                 // every jack, in file order
let shown = [];               // what the middle column currently lists
let probes = new Map();       // name -> { target, ms }
let group = null;             // selected tag path, null = all
let expanded = new Set();     // open tag paths
let sel = 0;
let palSel = 0;
let editing = null;   // jack name being edited, or null when adding
// Folders only exist because a jack carries the tag, so a brand-new empty one is
// held here until something lands in it. Dropped on reload, which is honest.
let pending = new Set();
let seeded = false;           // the tree's initial expansion is a one-off
let lastProbe = 0;            // epoch ms of the last sweep, for the throttle below
let vpns = new Map();         // folder path -> { up, known }
let vpnBusy = new Set();      // paths with an up/down command in flight
let detailMode = "jack";      // what the right pane describes: "jack" or "group"
let prefs = {};               // [settings] from the config
let providers = [];           // VPN presets this machine can drive
let colors = {};              // [colors] overrides, os key -> hex
let cfgPath = "";             // where the config lives, shown on first run
let sshKeys = [];             // private keys found in ~/.ssh, to suggest in the key field
let tunnels = [];             // live ssh -L forwards holding RDP open

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

// Lucide, inlined at generate time by scripts/icons.mjs — see ui/icons.js.
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
