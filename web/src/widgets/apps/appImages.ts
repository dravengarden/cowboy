// Images an app draws (docs/widget-apps.md §6). Every image of a version is
// loaded into a local blob before that version runs, so which images the app
// draws, and when, never causes a request: drawing is only a lookup by key.
// The Controller verifies each source at publish; this host checks the PNG
// header again before use (documents stored before that verification
// existed are never re-verified), and loads only same-origin artifacts and
// inline PNGs. Loaded sets are shared by every view of the same images and
// released a moment after the last one leaves.

import { createContext, useContext, useEffect, useState } from "react";
import type { ImageSource } from "../../../../contracts/widget-v1.ts";
import { pictureUrl } from "../WidgetImages";

export type AppImages = ReadonlyMap<string, string>;

export const AppImagesContext = createContext<AppImages>(new Map());

/** The local URL of the app image `key`, or null (the app shows `alt`). */
export function useAppImage(key: string): string | null {
  return useContext(AppImagesContext).get(key) ?? null;
}

const MAX_BYTES = 1024 * 1024;
const MAX_SIDE = 2048;
const MAX_PIXELS = 16 * 1024 * 1024;
const PNG = [137, 80, 78, 71, 13, 10, 26, 10];

/** Width × height of a still PNG within the app bounds, or null. Reads only
 *  chunk headers: the signature, IHDR, and that no `acTL` (animation)
 *  precedes the first IDAT. */
export function pngPixels(bytes: Uint8Array): number | null {
  if (bytes.length < 33 || bytes.length > MAX_BYTES) return null;
  if (PNG.some((value, index) => bytes[index] !== value)) return null;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const type = (at: number): string => String.fromCharCode(...bytes.subarray(at + 4, at + 8));
  if (type(8) !== "IHDR") return null;
  const width = view.getUint32(16);
  const height = view.getUint32(20);
  if (width === 0 || height === 0 || width > MAX_SIDE || height > MAX_SIDE) return null;
  for (let at = 8; at + 8 <= bytes.length; ) {
    const kind = type(at);
    if (kind === "acTL") return null;
    if (kind === "IDAT") return width * height;
    at += 12 + view.getUint32(at);
  }
  return null;
}

async function load(source: ImageSource): Promise<Uint8Array<ArrayBuffer> | null> {
  const url = pictureUrl(source, true);
  if (!url || (source.kind === "artifact" && !url.endsWith(".png"))) return null;
  if (source.kind === "data") {
    // Decoded here: the App CSP admits no fetch of a data: URL.
    return Uint8Array.from(atob(source.base64), (char) => char.charCodeAt(0));
  }
  const response = await fetch(url, { credentials: "same-origin" });
  if (!response.ok) return null;
  if (Number(response.headers.get("content-length") ?? "0") > MAX_BYTES) return null;
  return new Uint8Array(await response.arrayBuffer());
}

/** Checked bytes of every image of an app (full-DOM apps receive bytes over
 *  their port and make their own blobs). Images that fail to load or check,
 *  or exceed the app's pixel total, are absent. */
export async function loadAppImageBytes(images: unknown): Promise<Record<string, Uint8Array>> {
  const entries = Object.entries((images ?? {}) as Record<string, ImageSource>);
  const results = await Promise.all(
    entries.map(async ([key, source]) => [key, await load(source).catch(() => null)] as const),
  );
  const out: Record<string, Uint8Array> = {};
  let pixels = 0;
  for (const [key, bytes] of results) {
    const size = bytes ? pngPixels(bytes) : null;
    if (!bytes || size === null || pixels + size > MAX_PIXELS) continue;
    pixels += size;
    out[key] = bytes;
  }
  return out;
}

type Slot = {
  refs: number;
  images: AppImages | null;
  urls: string[];
  listeners: Set<() => void>;
  release: ReturnType<typeof setTimeout> | null;
};

const slots = new Map<string, Slot>();

function acquire(signature: string): Slot {
  let slot = slots.get(signature);
  if (!slot) {
    const created: Slot = { refs: 0, images: null, urls: [], listeners: new Set(), release: null };
    slot = created;
    slots.set(signature, created);
    const entries = Object.entries(JSON.parse(signature) as Record<string, ImageSource>);
    void Promise.all(
      entries.map(async ([key, source]) => [key, await load(source).catch(() => null)] as const),
    ).then((results) => {
      if (slots.get(signature) !== created) return;
      const map = new Map<string, string>();
      let pixels = 0;
      for (const [key, bytes] of results) {
        const size = bytes ? pngPixels(bytes) : null;
        // An image outside the bounds, or past the app's pixel total, stays
        // absent and its `Image` shows the alt text.
        if (!bytes || size === null || pixels + size > MAX_PIXELS) continue;
        pixels += size;
        const url = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
        created.urls.push(url);
        map.set(key, url);
      }
      created.images = map;
      for (const listener of created.listeners) listener();
    });
  }
  if (slot.release) clearTimeout(slot.release);
  slot.release = null;
  slot.refs += 1;
  return slot;
}

function release(signature: string): void {
  const slot = slots.get(signature);
  if (!slot) return;
  slot.refs -= 1;
  if (slot.refs > 0) return;
  // A moment later, so a view switching between versions never draws a
  // revoked blob.
  slot.release = setTimeout(() => {
    if (slot.refs > 0) return;
    slots.delete(signature);
    for (const url of slot.urls) URL.revokeObjectURL(url);
  }, 1000);
}

/** The loaded images of one version; null until every image has settled.
 *  An image that fails to load or check stays absent (its `Image` shows the
 *  alt text). */
export function useAppImagesLoaded(images: unknown): AppImages | null {
  const signature = JSON.stringify(images ?? {});
  const [, changed] = useState(0);
  useEffect(() => {
    const slot = acquire(signature);
    const listener = (): void => changed((count) => count + 1);
    slot.listeners.add(listener);
    if (slot.images) listener();
    return () => {
      slot.listeners.delete(listener);
      release(signature);
    };
  }, [signature]);
  return slots.get(signature)?.images ?? null;
}
