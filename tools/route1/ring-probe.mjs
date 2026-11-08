// Reads one pixel of a PNG (8-bit RGB/RGBA, non-interlaced, as Chrome writes them) without a dependency.
import fs from "node:fs";
import zlib from "node:zlib";

export function pixelR(file, x, y) {
  const b = fs.readFileSync(file);
  const w = b.readUInt32BE(16), h = b.readUInt32BE(20), bpp = b[25] === 6 ? 4 : 3;
  const chunks = [];
  for (let o = 8; o < b.length; ) {
    const len = b.readUInt32BE(o);
    if (b.toString("latin1", o + 4, o + 8) === "IDAT") chunks.push(b.subarray(o + 8, o + 8 + len));
    o += 12 + len;
  }
  const raw = zlib.inflateSync(Buffer.concat(chunks)), stride = w * bpp;
  let prev = Buffer.alloc(stride);
  for (let row = 0; row <= y; row++) {
    const f = raw[row * (stride + 1)], line = Buffer.from(raw.subarray(row * (stride + 1) + 1, (row + 1) * (stride + 1)));
    for (let i = 0; i < stride; i++) {
      const a = i >= bpp ? line[i - bpp] : 0, up = prev[i], c = i >= bpp ? prev[i - bpp] : 0;
      const p = a + up - c, pa = Math.abs(p - a), pb = Math.abs(p - up), pc = Math.abs(p - c);
      line[i] = (line[i] + (f === 0 ? 0 : f === 1 ? a : f === 2 ? up : f === 3 ? (a + up) >> 1 : pa <= pb && pa <= pc ? a : pb <= pc ? up : c)) & 255;
    }
    prev = line;
  }
  return { r: prev[x * bpp], w, h };
}
