// The sweep driver's own parts, with no browser and no network:
//   node --test tools/sweep/sweep.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import zlib from "node:zlib";

import { normalize, readPng, similarity, topCauses } from "./sweep.mjs";

// A PNG of `width` by `height`, RGB, each pixel `colour(x, y)`.
function png(width, height, colour) {
  const raw = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 3 + 1)] = 0;
    for (let x = 0; x < width; x++) {
      const [r, g, b] = colour(x, y);
      const o = y * (width * 3 + 1) + 1 + x * 3;
      raw[o] = r;
      raw[o + 1] = g;
      raw[o + 2] = b;
    }
  }
  const chunk = (name, data) => {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    return Buffer.concat([length, Buffer.from(name, "latin1"), data, Buffer.alloc(4)]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 2;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", zlib.deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

test("a PNG is read back as it was written", () => {
  const image = readPng(png(3, 2, (x, y) => [x * 10, y * 20, 7]));
  assert.equal(image.width, 3);
  assert.equal(image.height, 2);
  assert.deepEqual([...image.pixels.subarray(4 * 4, 4 * 4 + 4)], [10, 20, 7, 255]);
});

test("a page is like itself, and a blank page unlike one of text", () => {
  const text = readPng(png(128, 80, (x, y) => ((x >> 2) % 3 === 0 && y % 8 < 4 ? [0, 0, 0] : [255, 255, 255])));
  const blank = readPng(png(128, 80, () => [255, 255, 255]));
  assert.equal(similarity(text, text), 1);
  assert.ok(similarity(text, blank) < 0.8, String(similarity(text, blank)));
});

test("a console line's URLs, numbers and ids are taken out", () => {
  assert.equal(
    normalize("Uncaught TypeError: x at https://a.example/b.js?v=3:12:5 id 0123abcdef9"),
    "Uncaught TypeError: x at <url> id <id>",
  );
});

test("the causes are ranked by the sites they hit", () => {
  const rows = [
    { id: "a", silver: { counters: { missing: { MediaSource: 2 } }, console: [] } },
    { id: "b", silver: { counters: { missing: { MediaSource: 1, foo: 9 }, stackOverflows: 1 }, console: [] } },
  ];
  const causes = topCauses(rows);
  assert.equal(causes[0].key, "missing: MediaSource");
  assert.equal(causes[0].sites, 2);
  assert.equal(causes[0].total, 3);
  assert.ok(causes.some((c) => c.key === "stack: Maximum call stack" && c.sites === 1));
});
