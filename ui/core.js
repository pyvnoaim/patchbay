// Shared state and the helpers every other ui/ script leans on.
// Loaded first: nothing here may reference the later files at top level.
// Separate file, not an inline <script>, so the CSP can stay `script-src 'self'`.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const treeEl = $("tree"), listEl = $("list"), detailEl = $("d-body");
const searchBtn = $("searchbtn");
const paletteEl = $("palette"), pq = $("pq"), presultsEl = $("presults");
const ctxEl = $("ctx");
const sheetWrap = $("sheetwrap"), jackForm = $("jackform"), jfErr = $("jf-err"), jfDelete = $("jf-delete");
const askWrap = $("askwrap"), askForm = $("askform"), askInput = $("ask-input"), askErr = $("ask-err");

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
const hit = (j, f) =>
  !f || j.name.toLowerCase().includes(f) || j.host.toLowerCase().includes(f) ||
  (j.desc ?? "").toLowerCase().includes(f) || j.tags.some((t) => t.toLowerCase().includes(f));
