import { describe, expect, test } from "bun:test";
import { pngPixels } from "./appImages";

function chunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  new DataView(out.buffer).setUint32(0, data.length);
  out.set(new TextEncoder().encode(type), 4);
  out.set(data, 8);
  return out; // CRC is not read by the header check
}

function png(width: number, height: number, animated = false): Uint8Array {
  const header = new Uint8Array(13);
  new DataView(header.buffer).setUint32(0, width);
  new DataView(header.buffer).setUint32(4, height);
  const parts = [
    new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", header),
    ...(animated ? [chunk("acTL", new Uint8Array(8))] : []),
    chunk("IDAT", new Uint8Array(4)),
  ];
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

describe("app image header check", () => {
  test("still PNGs within 2048 × 2048 report their pixels", () => {
    expect(pngPixels(png(300, 200))).toBe(60_000);
    expect(pngPixels(png(2048, 2048))).toBe(2048 * 2048);
  });

  test("oversized, animated and non-PNG images are refused", () => {
    expect(pngPixels(png(8192, 8192))).toBe(null);
    expect(pngPixels(png(16, 16, true))).toBe(null);
    expect(pngPixels(new TextEncoder().encode("GIF89a, not a png at all, padded out"))).toBe(null);
    expect(pngPixels(new Uint8Array(1024 * 1024 + 1))).toBe(null);
  });
});
