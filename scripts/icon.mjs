// Draws the 1024px source icon: two patch-bay jacks joined by a cable.
// `npm run icon` renders this, then hands it to `tauri icon` for the .icns/.ico set.
// ponytail: 40 lines of SDF beats adding a canvas/image dependency for one PNG.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";

const S = 1024;

// --- minimal PNG writer ---------------------------------------------------
const T = new Int32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  T[n] = c;
}
const crc = (b) => {
  let c = ~0;
  for (const x of b) c = T[(c ^ x) & 0xff] ^ (c >>> 8);
  return ~c >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type, "latin1"), data]);
  const c = Buffer.alloc(4);
  c.writeUInt32BE(crc(td));
  return Buffer.concat([len, td, c]);
};
function png(w, h, rgba) {
  const stride = w * 4 + 1;
  const raw = Buffer.alloc(stride * h);
  for (let y = 0; y < h; y++) rgba.copy(raw, y * stride + 1, y * w * 4, (y + 1) * w * 4);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 6; // 8-bit RGBA
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// --- signed distance fields ----------------------------------------------
const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
const smooth = (e0, e1, x) => {
  const t = clamp((x - e0) / (e1 - e0), 0, 1);
  return t * t * (3 - 2 * t);
};
const cover = (d) => smooth(0.8, -0.8, d); // 1.6px of antialiasing

const sdRoundRect = (x, y, cx, cy, hw, hh, r) => {
  const qx = Math.abs(x - cx) - (hw - r);
  const qy = Math.abs(y - cy) - (hh - r);
  return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - r;
};
const sdRing = (x, y, cx, cy, r, half) => Math.abs(Math.hypot(x - cx, y - cy) - r) - half;
const sdDisc = (x, y, cx, cy, r) => Math.hypot(x - cx, y - cy) - r;

// A 2x2 bay of jacks; one patch cable runs corner to corner between two of them.
// (Two ports side by side plus a dipping cable reads as a smiley face - don't.)
const PORTS = [[386, 386], [638, 386], [386, 638], [638, 638]];
const A = PORTS[0], B = PORTS[3], C = [430, 606];
const CURVE = Array.from({ length: 96 }, (_, i) => {
  const t = i / 95, u = 1 - t;
  return [u * u * A[0] + 2 * u * t * C[0] + t * t * B[0], u * u * A[1] + 2 * u * t * C[1] + t * t * B[1]];
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
    let r = 0, g = 0, b = 0, a = 0;
    const over = (cr, cg, cb, ca) => {
      if (ca <= 0) return;
      const na = ca + a * (1 - ca);
      r = (cr * ca + r * a * (1 - ca)) / na;
      g = (cg * ca + g * a * (1 - ca)) / na;
      b = (cb * ca + b * a * (1 - ca)) / na;
      a = na;
    };

    // plate: indigo → blue, top-left lit
    const plate = cover(sdRoundRect(x, y, S / 2, S / 2, 432, 432, 208));
    const t = clamp((x + y) / (2 * S), 0, 1);
    over(0x2b + (0x60 - 0x2b) * (1 - t), 0x3f + (0x8e - 0x3f) * (1 - t), 0xd4 + (0xf6 - 0xd4) * (1 - t), plate);

    // cable first, then the jacks sit on top of it
    over(255, 255, 255, cover(sdCable(x, y, 19)) * 0.6);
    for (const [cx, cy] of PORTS) {
      over(255, 255, 255, cover(sdRing(x, y, cx, cy, 74, 21)));
      // the two the cable runs between are plugged in
      const plugged = (cx === A[0] && cy === A[1]) || (cx === B[0] && cy === B[1]);
      if (plugged) over(255, 255, 255, cover(sdDisc(x, y, cx, cy, 30)));
    }

    const i = (y * S + x) * 4;
    buf[i] = Math.round(r);
    buf[i + 1] = Math.round(g);
    buf[i + 2] = Math.round(b);
    buf[i + 3] = Math.round(a * 255);
  }
}

mkdirSync("assets", { recursive: true });
writeFileSync("assets/icon.png", png(S, S, buf));
console.log("wrote assets/icon.png");
