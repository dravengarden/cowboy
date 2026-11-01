/** Native image elements cannot attach authentication headers. This lifetime
 * loads only protected Cowboy resources through the authenticated fetch path. */
export function isProtectedImageSource(source: string): boolean {
  try {
    const url = new URL(source, globalThis.location.href);
    return url.origin === globalThis.location.origin && url.pathname.startsWith("/api/");
  } catch {
    return false;
  }
}

// --- Remote images -----------------------------------------------------------
// Agent output, tool results and repository Markdown choose image URLs. Fetching
// one needs no click, so its query string would carry anything an injected
// prompt read out of the browser, past an agent's network sandbox. The document
// CSP therefore allows no remote image at all (`img-src 'self' data: blob:`).
// That one rule also covers sinks no component gate can reach, such as `<img>`
// labels and image nodes inside a Mermaid diagram. A reader who wants a remote
// image opens it outside the app, where its origin is its own.

const LOCAL_IMAGE_SCHEMES = new Set(["data:", "blob:"]);

/** The host a source would contact, or `null` when it stays on this device or
 * origin. An unparseable source is remote with an empty host. */
export function remoteImageHost(source: string): string | null {
  try {
    const url = new URL(source, globalThis.location.href);
    if (LOCAL_IMAGE_SCHEMES.has(url.protocol)) return null;
    if (url.origin === globalThis.location.origin) return null;
    return url.host;
  } catch {
    return "";
  }
}

// --- Authenticated bytes -----------------------------------------------------

const RASTER_IMAGE = /^image\/(?:png|jpeg|gif|webp|avif|bmp|x-icon|vnd\.microsoft\.icon)$/;
const BASE64_CHUNK_BYTES = 0x8000;

export interface ImageDisplayUrl {
  url: string;
  release: () => void;
}

/** A URL for fetched bytes that is safe if the reader opens it as a page. A blob
 * URL has this origin, and Chrome does not apply the creator's CSP to a
 * browser-initiated navigation such as "Open image in new tab", so a repository
 * SVG would run script here. A `data:` document has an opaque origin. Only
 * formats that cannot carry script get the cheaper blob URL. */
export async function imageDisplayUrl(blob: Blob): Promise<ImageDisplayUrl> {
  const type = (blob.type.split(";")[0] ?? "").trim().toLowerCase();
  if (RASTER_IMAGE.test(type)) {
    const url = URL.createObjectURL(blob);
    return { url, release: () => URL.revokeObjectURL(url) };
  }
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  for (let index = 0; index < bytes.length; index += BASE64_CHUNK_BYTES) {
    binary += String.fromCharCode(...bytes.subarray(index, index + BASE64_CHUNK_BYTES));
  }
  return { url: `data:${type || "application/octet-stream"};base64,${btoa(binary)}`, release: () => {} };
}

export function setProtectedImageSource(image: HTMLImageElement, source: string): () => void {
  if (!isProtectedImageSource(source)) {
    if (remoteImageHost(source) === null) image.src = source;
    return () => {};
  }
  const url = new URL(source, globalThis.location.href);
  const controller = new AbortController();
  let display: ImageDisplayUrl | undefined;
  image.removeAttribute("src");
  void fetch(url, { credentials: "same-origin", signal: controller.signal }).then(async (response) => {
    if (!response.ok) throw new Error("Image access failed");
    const blob = await response.blob();
    if (controller.signal.aborted) return;
    const prepared = await imageDisplayUrl(blob);
    if (controller.signal.aborted) {
      prepared.release();
      return;
    }
    display = prepared;
    image.src = prepared.url;
  }).catch(() => {
    if (!controller.signal.aborted) image.dispatchEvent(new Event("error"));
  });
  return () => {
    controller.abort();
    image.removeAttribute("src");
    display?.release();
  };
}
