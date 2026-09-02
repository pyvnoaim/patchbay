// A PNG writer and the signed-distance helpers the artwork scripts share.
// ponytail: 40 lines of SDF beats adding a canvas/image dependency for two PNGs.
import { deflateSync } from "node:zlib";

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
export function png(w, h, rgba) {
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
export const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
export const smooth = (e0, e1, x) => {
  const t = clamp((x - e0) / (e1 - e0), 0, 1);
  return t * t * (3 - 2 * t);
};
export const cover = (d) => smooth(0.8, -0.8, d); // 1.6px of antialiasing

export const sdRoundRect = (x, y, cx, cy, hw, hh, r) => {
  const qx = Math.abs(x - cx) - (hw - r);
  const qy = Math.abs(y - cy) - (hh - r);
  return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - r;
};
export const sdRing = (x, y, cx, cy, r, half) => Math.abs(Math.hypot(x - cx, y - cy) - r) - half;
export const sdDisc = (x, y, cx, cy, r) => Math.hypot(x - cx, y - cy) - r;
/// Distance to the segment a→b, so a capsule of half-width `half` is `- half`.
export const sdSegment = (x, y, ax, ay, bx, by) => {
  const dx = bx - ax, dy = by - ay;
  const t = clamp(((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy), 0, 1);
  return Math.hypot(x - ax - t * dx, y - ay - t * dy);
};

/// Paint `src` over the accumulator `p` = [r, g, b, a], all 0..1 for alpha.
export function over(p, cr, cg, cb, ca) {
  if (ca <= 0) return;
  const na = ca + p[3] * (1 - ca);
  p[0] = (cr * ca + p[0] * p[3] * (1 - ca)) / na;
  p[1] = (cg * ca + p[1] * p[3] * (1 - ca)) / na;
  p[2] = (cb * ca + p[2] * p[3] * (1 - ca)) / na;
  p[3] = na;
}
