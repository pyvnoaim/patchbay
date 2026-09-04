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
import { png, clamp, cover, smooth, over, sdRing, sdRoundRect } from "./draw.mjs";

const W = 660, H = 400;
const APP = [176, 190], APPS = [484, 190];
// Finder draws the icon at 128 and the label under it, so the artwork stays outside
// that square or it is simply covered up.
const CLEAR = 74;

// The one instruction the window carries: drag left to right. A patch lead: a plug
// at each end, a cable with a little slack between them. Plugs and not the icon's
// ring sockets, and barely any sag - two rings over a dipping curve is a smiley face,
// which is the same trap icon.mjs names. Between the icons only, so none of it hides
// beneath either.
// A plug is a barrel lying along the cable, its tip toward the icon it goes into.
const BARREL = 9, BODY = 3.6, TIPLEN = 5, TIPW = 2.2, PLUG = 2 * (BARREL + TIPLEN);
const TAIL = [APP[0] + CLEAR + PLUG, APP[1]], TIP = [APPS[0] - CLEAR - PLUG, APPS[1]];
const SAG = 9, HALF = 2.1;
// The arc through both plugs that dips SAG below them: one circle, clipped to the
// span between them and to its lower half.
const MID = (TAIL[0] + TIP[0]) / 2, L = TIP[0] - TAIL[0];
const R = (L * L / 4 + SAG * SAG) / (2 * SAG), CY = TAIL[1] + SAG - R;

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

    // The cable, a shade lighter at the tail so the eye still travels left to right
    // without an arrowhead saying so. Under the sockets, the way the icon layers it.
    if (y > CY && x >= TAIL[0] && x <= TIP[0]) {
      const along = (x - TAIL[0]) / L;
      over(p, 0x3d, 0x6f, 0xe0, cover(sdRing(x, y, MID, CY, R, HALF)) * (0.5 + 0.35 * along));
    }
    for (const [[sx, sy], dir] of [[TAIL, -1], [TIP, 1]]) {
      const body = sdRoundRect(x, y, sx + dir * BARREL, sy, BARREL, BODY, 2);
      const tip = sdRoundRect(x, y, sx + dir * (2 * BARREL + TIPLEN), sy, TIPLEN, TIPW, 1.5);
      over(p, 0x3d, 0x6f, 0xe0, cover(Math.min(body, tip)) * 0.85);
    }

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
