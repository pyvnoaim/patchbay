// Draws the .dmg window's background; the bundler picks it up from `bundle.macOS.dmg.background`.
// The icon positions must match tauri.conf.json: Finder's coordinates and this canvas share an origin.
// Light, not dark: Finder draws a .dmg's icon labels in black whatever the appearance.
// Drawn at 1x and 2x and joined into one .tiff by tiffutil, which is how Finder is told a
// background has a retina half - a bare PNG has no way to say so.
import { writeFileSync, mkdirSync, mkdtempSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { png, clamp, cover, smooth, over, sdRing, sdRoundRect } from "./draw.mjs";

const W = 660,
  H = 400;
const APP = [176, 190],
  APPS = [484, 190];
// Finder draws the icon at 128 with the label under it, so the artwork stays outside that square.
const CLEAR = 74;

// The one instruction the window carries: drag left to right, drawn as a patch lead between the
// icons. A plug is a barrel lying along the cable, its tip toward the icon it goes into.
const BARREL = 9,
  BODY = 3.6,
  TIPLEN = 5,
  TIPW = 2.2,
  PLUG = 2 * (BARREL + TIPLEN);
const TAIL = [APP[0] + CLEAR + PLUG, APP[1]],
  TIP = [APPS[0] - CLEAR - PLUG, APPS[1]];
const SAG = 9,
  HALF = 2.1;
// The arc through both plugs that dips SAG below them: one circle, clipped to its lower half.
const MID = (TAIL[0] + TIP[0]) / 2,
  L = TIP[0] - TAIL[0];
const R = ((L * L) / 4 + SAG * SAG) / (2 * SAG),
  CY = TAIL[1] + SAG - R;

function render(S) {
  const buf = Buffer.alloc(W * S * H * S * 4);
  // Distances are in points; the antialiasing band stays 1.6 device pixels wide.
  const aa = (d) => cover(d * S);
  for (let py = 0; py < H * S; py++) {
    for (let px = 0; px < W * S; px++) {
      const x = (px + 0.5) / S,
        y = (py + 0.5) / S;
      const p = [0, 0, 0, 0];

      // Paper, cooler at the foot, with the icon's indigo bled in behind it.
      const v = y / H;
      over(p, 0xfb - 14 * v, 0xfb - 14 * v, 0xfd - 12 * v, 1);
      over(p, 0x3d, 0x5f, 0xe8, 0.07 * smooth(400, 40, Math.hypot(x - APP[0], y - APP[1])));
      // Corners eased down, so the window has an edge without a border.
      over(p, 0x2a, 0x2a, 0x38, 0.05 * smooth(0.55, 1.0, Math.hypot(x / W - 0.5, y / H - 0.5) * 2));

      // The cable, lighter at the tail so the eye travels left to right. Under the plugs.
      if (y > CY && x >= TAIL[0] && x <= TIP[0]) {
        const along = (x - TAIL[0]) / L;
        over(p, 0x3d, 0x6f, 0xe0, aa(sdRing(x, y, MID, CY, R, HALF)) * (0.5 + 0.35 * along));
      }
      for (const [[sx, sy], dir] of [
        [TAIL, -1],
        [TIP, 1],
      ]) {
        const body = sdRoundRect(x, y, sx + dir * BARREL, sy, BARREL, BODY, 2);
        const tip = sdRoundRect(x, y, sx + dir * (2 * BARREL + TIPLEN), sy, TIPLEN, TIPW, 1.5);
        over(p, 0x3d, 0x6f, 0xe0, aa(Math.min(body, tip)) * 0.85);
      }

      const i = (py * W * S + px) * 4;
      buf[i] = Math.round(clamp(p[0], 0, 255));
      buf[i + 1] = Math.round(clamp(p[1], 0, 255));
      buf[i + 2] = Math.round(clamp(p[2], 0, 255));
      buf[i + 3] = Math.round(p[3] * 255);
    }
  }
  return png(W * S, H * S, buf);
}

const tmp = mkdtempSync(join(tmpdir(), "patchbay-dmg-"));
const one = join(tmp, "bg.png"),
  two = join(tmp, "bg@2x.png");
writeFileSync(one, render(1));
writeFileSync(two, render(2));
mkdirSync("assets", { recursive: true });
// -cathidpicheck refuses the pair unless the second really is twice the first.
execFileSync("tiffutil", ["-cathidpicheck", one, two, "-out", "assets/dmg-background.tiff"]);
console.log("wrote assets/dmg-background.tiff (1x and 2x)");
