// Draws the 1024px source icon: two patch-bay jacks joined by a cable.
// `npm run icon` renders this, then hands it to `tauri icon` for the .icns/.ico set.
import { writeFileSync, mkdirSync } from "node:fs";
import { png, clamp, cover, over, sdRoundRect, sdRing, sdDisc } from "./draw.mjs";

const S = 1024;

// A 2x2 bay of jacks; one patch cable runs corner to corner between two of them.
// (Two ports side by side plus a dipping cable reads as a smiley face - don't.)
const PORTS = [
  [386, 386],
  [638, 386],
  [386, 638],
  [638, 638],
];
const A = PORTS[0],
  B = PORTS[3],
  C = [430, 606];
const CURVE = Array.from({ length: 96 }, (_, i) => {
  const t = i / 95,
    u = 1 - t;
  return [
    u * u * A[0] + 2 * u * t * C[0] + t * t * B[0],
    u * u * A[1] + 2 * u * t * C[1] + t * t * B[1],
  ];
});
function sdCable(x, y, half) {
  let best = Infinity;
  for (const [px, py] of CURVE) best = Math.min(best, Math.hypot(x - px, y - py));
  return best - half;
}

// --- compose --------------------------------------------------------------
const buf = Buffer.alloc(S * S * 4);
for (let y = 0; y < S; y++) {
  for (let x = 0; x < S; x++) {
    const p = [0, 0, 0, 0];

    // plate: indigo → blue, top-left lit
    const plate = cover(sdRoundRect(x, y, S / 2, S / 2, 432, 432, 208));
    const t = clamp((x + y) / (2 * S), 0, 1);
    over(
      p,
      0x2b + (0x60 - 0x2b) * (1 - t),
      0x3f + (0x8e - 0x3f) * (1 - t),
      0xd4 + (0xf6 - 0xd4) * (1 - t),
      plate,
    );

    // cable first, then the jacks sit on top of it
    over(p, 255, 255, 255, cover(sdCable(x, y, 19)) * 0.6);
    for (const [cx, cy] of PORTS) {
      over(p, 255, 255, 255, cover(sdRing(x, y, cx, cy, 74, 21)));
      // the two the cable runs between are plugged in
      const plugged = (cx === A[0] && cy === A[1]) || (cx === B[0] && cy === B[1]);
      if (plugged) over(p, 255, 255, 255, cover(sdDisc(x, y, cx, cy, 30)));
    }

    const i = (y * S + x) * 4;
    buf[i] = Math.round(p[0]);
    buf[i + 1] = Math.round(p[1]);
    buf[i + 2] = Math.round(p[2]);
    buf[i + 3] = Math.round(p[3] * 255);
  }
}

mkdirSync("assets", { recursive: true });
writeFileSync("assets/icon.png", png(S, S, buf));
console.log("wrote assets/icon.png");
