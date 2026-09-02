// Draws the .dmg window's background. `npm run dmg` renders it; the bundler picks
// it up from `bundle.macOS.dmg.background`, and the icon positions below must match
// the ones in tauri.conf.json - Finder's coordinates and this canvas share an origin.
//
// Light, not dark, and that is not a taste call: Finder draws a .dmg's icon labels
// in black whatever appearance the machine is in, so "patchbay" and "Applications"
// are unreadable on a dark ground.
//
// ponytail: 1x only, so it is a touch soft on a retina display. A .tiff with both
// representations is the fix if anyone minds.
import { writeFileSync, mkdirSync } from "node:fs";
import { png, clamp, cover, smooth, over, sdSegment } from "./draw.mjs";

const W = 660, H = 400;
const APP = [176, 190], APPS = [484, 190];
// Finder draws the icon at 128 and the label under it, so the artwork stays outside
// that square or it is simply covered up.
const CLEAR = 74;

// The one instruction the window carries: drag left to right. Tail to head only,
// between the two icons, so none of it hides beneath either.
const TAIL = [APP[0] + CLEAR, APP[1]], TIP = [APPS[0] - CLEAR, APPS[1]];
const WING = 11, SPREAD = 7;

const buf = Buffer.alloc(W * H * 4);
for (let y = 0; y < H; y++) {
  for (let x = 0; x < W; x++) {
    const p = [0, 0, 0, 0];

    // Paper, a shade cooler at the foot, with the app icon's indigo bled in behind
    // it so the window is tinted rather than painted.
    const v = y / H;
    over(p, 0xfb - 14 * v, 0xfb - 14 * v, 0xfd - 12 * v, 1);
    over(p, 0x3d, 0x5f, 0xe8, 0.07 * smooth(400, 40, Math.hypot(x - APP[0], y - APP[1])));
    // and the corners eased down, so the window has an edge without a border
    over(p, 0x2a, 0x2a, 0x38, 0.05 * smooth(0.55, 1.0, Math.hypot(x / W - 0.5, y / H - 0.5) * 2));

    // The shaft fades in from nothing: an arrow that starts abruptly reads as a
    // line someone forgot to finish.
    const along = clamp((x - TAIL[0]) / (TIP[0] - TAIL[0]), 0, 1);
    const shaft = cover(sdSegment(x, y, TAIL[0], TAIL[1], TIP[0], TIP[1]) - 1.4);
    over(p, 0x3d, 0x6f, 0xe0, shaft * (0.16 + 0.64 * along));

    const head = Math.min(
      sdSegment(x, y, TIP[0], TIP[1], TIP[0] - WING, TIP[1] - SPREAD),
      sdSegment(x, y, TIP[0], TIP[1], TIP[0] - WING, TIP[1] + SPREAD),
    );
    over(p, 0x3d, 0x6f, 0xe0, cover(head - 1.4) * 0.8);

    const i = (y * W + x) * 4;
    buf[i] = Math.round(clamp(p[0], 0, 255));
    buf[i + 1] = Math.round(clamp(p[1], 0, 255));
    buf[i + 2] = Math.round(clamp(p[2], 0, 255));
    buf[i + 3] = Math.round(p[3] * 255);
  }
}

mkdirSync("assets", { recursive: true });
writeFileSync("assets/dmg-background.png", png(W, H, buf));
console.log("wrote assets/dmg-background.png");
