// Turn a browser `File` (picked from disk or pasted from the clipboard) into an
// ACP content block the daemon forwards verbatim to the agent. The wire path
// already exists end-to-end: the composer sends `Inbound::Prompt { content }`
// (see src/core.rs — the `content` array was added precisely for "pasted
// images"), server.rs parses each entry as an `agent_client_protocol::
// ContentBlock`, and the agent receives them in the prompt turn.
//
// Three block shapes, by file kind:
//   - image/*            → `ContentBlock::Image { data, mimeType }` — the model
//                          *sees* the picture (screenshots, photos, diagrams).
//   - text-ish files     → `ContentBlock::Resource` carrying TextResourceContents
//                          ({ uri, text, mimeType }) — the model reads the source.
//   - everything else    → `ContentBlock::Resource` carrying BlobResourceContents
//                          ({ uri, blob, mimeType }) — base64, model decides.
//
// Why embed rather than upload-to-disk + ResourceLink: the file lives on the
// *client* (a phone, a laptop), not on hawk where the agent's working dir is.
// A path-based ResourceLink would dangle. Embedding ships the bytes inline, so
// it works from any device with no server-side upload endpoint or scratch dir.

import type { ContentBlock } from "./protocol";
import { newUuid } from "./uuid";

/// A staged attachment: the ACP block to send plus the metadata the composer
/// needs to render a removable preview chip / thumbnail before sending.
export interface Attachment {
  /** Local key for React lists + remove. */
  id: string;
  /** Display name (the file name, or a synthesized one for clipboard images). */
  name: string;
  mimeType: string;
  /** True for `image/*` — the composer shows a thumbnail instead of a file chip. */
  isImage: boolean;
  /** `data:` URL for the image thumbnail; absent for non-image files. */
  previewUrl?: string;
  /** The ACP `ContentBlock` JSON sent inside `Inbound::Prompt { content }`. */
  block: ContentBlock;
  /** True only while a pasted image is being encoded. It may render locally,
   * but must not be sent or persisted as a completed attachment yet. */
  pending?: boolean;
}

/** Read every file representation exposed by a clipboard paste. iOS WebKit can
 * expose a pasted photo through `items` while leaving `files` empty; desktop
 * browsers commonly populate both, so dedupe by File identity. */
export function clipboardFiles(
  clipboard: Pick<DataTransfer, "files" | "items">,
): File[] {
  const files = Array.from(clipboard.files);
  const seen = new Set(files);
  for (const item of Array.from(clipboard.items)) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (file && !seen.has(file)) {
      seen.add(file);
      files.push(file);
    }
  }
  return files;
}

// Text-ish MIME types we embed as readable source (TextResourceContents) rather
// than base64 blobs — so the model gets the actual characters, not bytes it has
// to decode. Covers the common code / config / data formats.
const TEXT_MIME_RE =
  /^(text\/|application\/(json|xml|javascript|ecmascript|x-sh|x-shellscript|x-yaml|yaml|toml|x-toml|markdown|x-httpd-php|graphql|x-www-form-urlencoded))/i;

// Extensions we treat as text when the browser reports no (or a useless) MIME —
// common on code files dragged in from a phone / certain OSes.
const TEXT_EXT = new Set([
  "txt", "md", "markdown", "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "json",
  "jsonc", "toml", "yaml", "yml", "py", "go", "c", "h", "cpp", "hpp", "cc",
  "java", "kt", "rb", "php", "sh", "bash", "fish", "zsh", "sql", "css", "scss",
  "html", "htm", "xml", "svg", "csv", "tsv", "ini", "cfg", "conf", "env", "lua",
  "nix", "cue", "proto", "graphql", "gql", "vue", "svelte", "diff", "patch",
  "dockerfile", "makefile", "lock", "log",
]);

export function nextAttachmentId(): string {
  // A GLOBALLY-unique id, not a reload-resettable counter. Inline-image tokens
  // `![](cowboy-att:<id>)` persist inside restored drafts / queued messages and
  // keep their original ids (blocksToAttachments reuses them). A module-level
  // counter resets to 0 on every page reload, so the FIRST fresh paste after a
  // reload minted `att1` — colliding with a restored `att1` and overwriting its
  // bytes in the inline-image registry (which is keyed by id): the first image
  // silently became the newly-pasted one ("末尾粘贴图片覆盖第一张"). A UUID can never
  // collide with a restored id. The helper also covers older WKWebView and HTTP
  // development origins where randomUUID is unavailable.
  return `att-${newUuid()}`;
}

/** Create an immediately renderable image placeholder during the native paste
 * event. This lets React promote the focused textarea to CM6 in the same UIKit
 * gesture; the expensive payload encoding finishes asynchronously. */
export function pendingImageAttachment(file: File, id = nextAttachmentId()): Attachment {
  const mimeType = file.type || "image/png";
  return {
    id,
    name: file.name || "pasted-image",
    mimeType,
    isImage: true,
    previewUrl: URL.createObjectURL(file),
    block: { type: "image", data: "", mimeType },
    pending: true,
  };
}

/** Resolve one asynchronous paste batch without touching a newer pending batch. */
export function settlePendingAttachments(
  current: readonly Attachment[],
  pendingBatch: readonly Attachment[],
  completedBatch: readonly Attachment[],
): Attachment[] {
  const pendingIds = new Set(pendingBatch.map((attachment) => attachment.id));
  const completedById = new Map(
    completedBatch.map((attachment) => [attachment.id, attachment]),
  );
  return current.flatMap((attachment) => {
    if (!pendingIds.has(attachment.id)) return [attachment];
    const replacement = completedById.get(attachment.id);
    return replacement ? [replacement] : [];
  });
}

function extOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
}

// Read a File as a base64 string (no `data:…;base64,` prefix). Goes through
// readAsDataURL — the browser's only built-in base64 path — then strips the
// prefix. Rejects on read error so the caller can drop the attachment.
function readBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = (): void => reject(reader.error ?? new Error("read failed"));
    reader.onload = (): void => {
      const result = typeof reader.result === "string" ? reader.result : "";
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.readAsDataURL(file);
  });
}

function readText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = (): void => reject(reader.error ?? new Error("read failed"));
    reader.onload = (): void => resolve(typeof reader.result === "string" ? reader.result : "");
    reader.readAsText(file);
  });
}

function isTextual(file: File): boolean {
  if (TEXT_MIME_RE.test(file.type)) return true;
  // No / generic MIME: fall back to the extension (and the bare `Makefile` /
  // `Dockerfile` style names that have no extension at all).
  if (file.type === "" || file.type === "application/octet-stream") {
    const ext = extOf(file.name);
    if (ext && TEXT_EXT.has(ext)) return true;
    const base = file.name.toLowerCase();
    if (base === "dockerfile" || base === "makefile") return true;
  }
  return false;
}

// A stable-ish uri for the embedded resource. The bytes travel inline, so this
// is just an identifier the agent echoes / labels the resource by — there is no
// real file at this path. `attachment:` keeps it from being mistaken for a
// readable host path.
function attachmentUri(name: string): string {
  return `attachment:///${encodeURIComponent(name)}`;
}

// One re-encoded raster: base64 (no prefix) for the wire + a `data:` URL for an
// <img>, plus the (re-encoded) mime.
interface Raster {
  base64: string;
  dataUrl: string;
  mimeType: string;
}

// Draw a bitmap into a capped-size JPEG. The single expensive step (decoding the
// full-resolution source) already happened in createImageBitmap; this just
// rasters a small canvas, so it's cheap even for a 12MP source.
function drawScaled(bmp: ImageBitmap, maxEdge: number, quality: number): Raster {
  const scale = Math.min(1, maxEdge / Math.max(bmp.width, bmp.height));
  const w = Math.max(1, Math.round(bmp.width * scale));
  const h = Math.max(1, Math.round(bmp.height * scale));
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.drawImage(bmp, 0, 0, w, h);
  const dataUrl = canvas.toDataURL("image/jpeg", quality);
  const comma = dataUrl.indexOf(",");
  return { dataUrl, base64: comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl, mimeType: "image/jpeg" };
}

// Downscale a picked/pasted image to ONE capped raster (from one decode). Why:
// a phone photo is several MB; embedding it verbatim — and decoding the full
// raster just to paint a preview — is what froze the composer on pick and bloated
// the WS frame on send. createImageBitmap decodes once off the main thread; we
// draw a single ≤1568px q0.85 JPEG and use it for BOTH the agent payload and the
// preview:
//   - 1568px is Claude's vision sweet spot — larger is downsampled server-side,
//     so full-res only wastes bytes + CPU on the send.
//   - the SAME raster is `previewUrl`, which drives every user-facing display (the
//     inline composer image, queue/draft rows, AND the lightbox's "open full" —
//     all read `previewUrl`; see inlineImages.ts / ResourceLightbox.tsx). It was a
//     separate 256px thumb before, which is below even the inline image's ~360px
//     CSS box (≈720–1080px on a 2–3× retina screen) — a pasted screenshot looked
//     badly blurred. 1568 is sharp there AND exactly matches what a sent message
//     shows (blocksToAttachments rebuilds previewUrl from the 1568 send bytes), so
//     there's no quality jump on send. The send dataUrl is already computed, so
//     reusing it for the preview is free (vs the old second thumb raster).
// Returns null when the browser can't rasterize (no canvas / decode failure);
// the caller then falls back to embedding the original bytes verbatim.
async function encodeImage(file: File): Promise<Raster | null> {
  try {
    // Bound the decode. iOS Safari's createImageBitmap can hang indefinitely on
    // some images (e.g. right after the native photo picker closes); an
    // unsettled promise here would leave filesToAttachments' Promise.allSettled
    // pending forever, so the attachment never stages and no preview ever
    // appears. On timeout we resolve null and fall back to embedding the raw
    // bytes — the chip/preview still shows.
    const bmp = await Promise.race([
      createImageBitmap(file),
      new Promise<null>((resolve) => setTimeout(() => resolve(null), 4000)),
    ]);
    if (!bmp) return null;
    try {
      return drawScaled(bmp, 1568, 0.85);
    } finally {
      bmp.close();
    }
  } catch {
    return null;
  }
}

/// Convert one picked / pasted file into a stageable `Attachment`. Throws (via a
/// rejected promise) only on a FileReader error — callers should `.catch` and
/// drop that single file rather than failing the whole batch.
export async function fileToAttachment(
  file: File,
  id = nextAttachmentId(),
): Promise<Attachment> {
  const mimeType = file.type || "application/octet-stream";
  if (mimeType.startsWith("image/")) {
    const encoded = await encodeImage(file);
    if (encoded) {
      return {
        id,
        name: file.name || "pasted-image",
        mimeType: encoded.mimeType,
        isImage: true,
        previewUrl: encoded.dataUrl,
        block: { type: "image", data: encoded.base64, mimeType: encoded.mimeType },
      };
    }
    // Fallback: the browser couldn't rasterize — embed the original bytes.
    const data = await readBase64(file);
    return {
      id,
      name: file.name || "pasted-image",
      mimeType,
      isImage: true,
      previewUrl: `data:${mimeType};base64,${data}`,
      block: { type: "image", data, mimeType },
    };
  }
  const uri = attachmentUri(file.name || "attachment");
  if (isTextual(file)) {
    const text = await readText(file);
    return {
      id,
      name: file.name || "attachment.txt",
      mimeType,
      isImage: false,
      block: { type: "resource", resource: { uri, text, mimeType } },
    };
  }
  const blob = await readBase64(file);
  return {
    id,
    name: file.name || "attachment",
    mimeType,
    isImage: false,
    block: { type: "resource", resource: { uri, blob, mimeType } },
  };
}

/// Convert a batch of files concurrently, dropping any that fail to read. Used
/// by both the file picker and the paste handler.
export async function filesToAttachments(files: readonly File[]): Promise<Attachment[]> {
  const settled = await Promise.allSettled(files.map((f) => fileToAttachment(f)));
  return settled
    .filter((r): r is PromiseFulfilledResult<Attachment> => r.status === "fulfilled")
    .map((r) => r.value);
}

// Derive a display name from an `attachment:///<name>` resource uri (the scheme
// fileToAttachment mints). Falls back to a generic label for anything else.
function nameFromUri(uri: string): string {
  const last = /([^/]+)\/?$/.exec(uri)?.[1];
  if (!last) return "attachment";
  try {
    return decodeURIComponent(last);
  } catch {
    return last;
  }
}

/// Reconstruct staged `Attachment`s from a queued message's ACP content blocks —
/// the inverse of `buildContentBlocks`, for rendering / re-editing a server-synced
/// queue or draft on a terminal that didn't compose it. Trailing `text` blocks
/// are skipped (the prompt text is shown from the message's own `text` field);
/// only `image` / `resource` blocks become attachments. The original block is
/// carried back verbatim so a re-send re-emits identical content.
export function blocksToAttachments(
  blocks: readonly ContentBlock[],
  text?: string,
): Attachment[] {
  // A reconstructed image attachment MUST keep the same id as the inline
  // `![](cowboy-att:<id>)` token already embedded in `text` — the inline-image
  // decoration resolves bytes by that id (inlineImages.ts registry), and a mismatch
  // makes it fall back to a chip ("🖼 image.png") instead of the thumbnail. The ids
  // are session-local counters (nextId), so a server round-trip / reload would
  // otherwise mint fresh ids no token references (the "synced draft shows chips"
  // bug). buildContentBlocks emits image blocks in DOCUMENT ORDER (tokened images
  // first, interleaved with text), so align the Nth image block to the Nth token.
  const tokenIds = text !== undefined ? imageTokensInText(text).map((t) => t.id) : [];
  let imageSeen = 0;
  const out: Attachment[] = [];
  for (const block of blocks) {
    if (block.type === "image") {
      const data = typeof block.data === "string" ? block.data : "";
      const mimeType = typeof block.mimeType === "string" ? block.mimeType : "image/jpeg";
      out.push({
        id: tokenIds[imageSeen++] ?? nextAttachmentId(),
        name: "image",
        mimeType,
        isImage: true,
        previewUrl: `data:${mimeType};base64,${data}`,
        block,
      });
    } else if (block.type === "resource") {
      const resource = (block.resource ?? {}) as { uri?: unknown; mimeType?: unknown };
      const uri = typeof resource.uri === "string" ? resource.uri : "";
      const mimeType =
        typeof resource.mimeType === "string" ? resource.mimeType : "application/octet-stream";
      out.push({
        id: nextAttachmentId(),
        name: nameFromUri(uri),
        mimeType,
        isImage: false,
        block,
      });
    }
    // `text` blocks are the trailing prompt text — rendered from message.text.
  }
  return out;
}

/// The inline-image token the composer writes into the literal doc at the paste
/// position: `![name](cowboy-att:<id>)`. The bytes live in the host's
/// `attachments[]` / the inline-image registry (see inlineImages.ts) — the doc
/// only carries this marker, which a decoration renders as a thumbnail. The `id`
/// matches an `Attachment.id`. Global flag for repeated `.exec`; callers MUST
/// reset `.lastIndex` (or use `String.matchAll`) before scanning.
export const IMG_TOKEN_RE = /!\[[^\]]*\]\(cowboy-att:([^)]+)\)/g;

/// Ordered (id, span) of every inline-image token in `text`, document order.
export function imageTokensInText(
  text: string,
): { id: string; from: number; to: number }[] {
  const out: { id: string; from: number; to: number }[] = [];
  for (const m of text.matchAll(IMG_TOKEN_RE)) {
    if (m[1] !== undefined) {
      out.push({ id: m[1], from: m.index, to: m.index + m[0].length });
    }
  }
  return out;
}

/** Recover images whose placement token was removed by the retired Mobile
 * textarea fallback. Their original position is unknowable, so append them in
 * attachment order and make that deterministic position authoritative again. */
export function promoteUnplacedImageTokens(
  text: string,
  attachments: readonly Attachment[],
): string {
  const placed = new Set(imageTokensInText(text).map((token) => token.id));
  const blocks = attachments
    .filter((attachment) => attachment.isImage && !placed.has(attachment.id))
    .map((attachment) => {
      const label = attachment.name.replaceAll("]", "");
      return `![${label}](cowboy-att:${attachment.id})`;
    });
  if (blocks.length === 0) return text;
  const base = text.trimEnd();
  return `${base ? `${base}\n` : ""}${blocks.join("\n")}\n`;
}

/**
 * Keep attachment bytes in lockstep with inline-image token deletion.
 *
 * Images and their `cowboy-att:` markers intentionally live in separate state.
 * A CodeMirror delete changes the document first; without this reconciliation,
 * saving a draft re-sends the stale image block and the server restores the
 * picture. Only images that had a token in the previous text are eligible for
 * removal, so legacy/gallery-only attachments and non-image files stay intact.
 */
export function reconcileDeletedInlineImages(
  previousText: string,
  nextText: string,
  attachments: Attachment[],
): Attachment[] {
  const previousIds = new Set(imageTokensInText(previousText).map((token) => token.id));
  // Ordinary text input is overwhelmingly the hot path. Preserve the array
  // identity when no inline image can have been removed so React state bails
  // out instead of scheduling an attachment update for every keystroke.
  if (previousIds.size === 0) return attachments;
  const nextIds = new Set(imageTokensInText(nextText).map((token) => token.id));
  const reconciled = attachments.filter((attachment) =>
    !attachment.isImage || !previousIds.has(attachment.id) || nextIds.has(attachment.id)
  );
  return reconciled.length === attachments.length ? attachments : reconciled;
}

/// Drop every inline-image token from `text` (for any plain-text view — the
/// stored message `text` field / row previews must never show a raw token).
/// Collapses the double space a removed mid-line token leaves behind.
export function stripImageTokens(text: string): string {
  return text.replace(IMG_TOKEN_RE, "").replace(/ {2,}/g, " ");
}

export type AttachmentDisplayPart =
  | { type: "text"; text: string }
  | { type: "attachment"; attachment: Attachment };

/**
 * Rebuild a local message's display order without turning image bytes into a
 * Markdown URL. react-markdown intentionally rejects `data:` image URLs, which
 * makes the browser paint only the image's alt text (for example `image.png`)
 * while an optimistic send is offline. Render attachments directly from the
 * message-owned data instead, so pending/failed/retry UI never depends on the
 * composer registry or the network.
 */
export function attachmentDisplayParts(
  text: string,
  attachments: readonly Attachment[],
): AttachmentDisplayPart[] {
  const tokens = imageTokensInText(text);
  if (tokens.length === 0) {
    return [
      ...attachments.map((attachment): AttachmentDisplayPart => ({
        type: "attachment",
        attachment,
      })),
      ...(text ? [{ type: "text" as const, text }] : []),
    ];
  }

  const byId = new Map(attachments.map((attachment) => [attachment.id, attachment]));
  const seen = new Set<string>();
  const parts: AttachmentDisplayPart[] = [];
  let cursor = 0;
  for (const token of tokens) {
    const segment = text.slice(cursor, token.from);
    if (segment) parts.push({ type: "text", text: segment });
    const attachment = byId.get(token.id);
    if (attachment) {
      parts.push({ type: "attachment", attachment });
      seen.add(attachment.id);
    }
    cursor = token.to;
  }
  const tail = text.slice(cursor);
  if (tail) parts.push({ type: "text", text: tail });
  for (const attachment of attachments) {
    if (!seen.has(attachment.id)) parts.push({ type: "attachment", attachment });
  }
  return parts;
}

/// Drop only the inline-image tokens whose bytes aren't in `ids` — used when
/// rehydrating a persisted draft, so a token left orphaned by a prior quota-drop
/// of its attachment doesn't reload as a stray fallback chip ("的样式 bug").
export function dropOrphanImageTokens(text: string, ids: ReadonlySet<string>): string {
  return text
    .replace(IMG_TOKEN_RE, (full, id: string) => (ids.has(id) ? full : ""))
    .replace(/ {2,}/g, " ");
}

/// Build the ACP `content` block array for a prompt.
///   • With inline tokens (Obsidian-style images): walk the doc and emit blocks
///     in DOCUMENT ORDER — text segment, image block, text segment, … — so the
///     agent sees images positioned exactly where the user placed them (Zed-
///     consistent). Tokens are stripped from the text segments; an image is
///     resolved by id from `attachments`, skipped if absent.
///   • No tokens (legacy path): attachment blocks first, then a trailing text
///     block — so the agent has the visual/file context before the instruction.
/// Returns null when there's nothing to send (callers use the text-only path).
export function buildContentBlocks(
  text: string,
  attachments: readonly Attachment[],
): ContentBlock[] | null {
  const tokens = imageTokensInText(text);
  if (tokens.length > 0) {
    const byId = new Map(attachments.map((a) => [a.id, a]));
    const seen = new Set<string>();
    const blocks: ContentBlock[] = [];
    let cursor = 0;
    for (const t of tokens) {
      const seg = text.slice(cursor, t.from);
      if (seg.trim()) blocks.push({ type: "text", text: seg });
      const att = byId.get(t.id);
      if (att) {
        blocks.push(att.block);
        seen.add(att.id);
      }
      cursor = t.to;
    }
    const tail = text.slice(cursor);
    if (tail.trim()) blocks.push({ type: "text", text: tail });
    // Any attachment WITHOUT an inline token (a non-image file kept in the tray, or
    // one added via a flow that didn't write a token) must still be sent — append
    // it so it's never silently dropped.
    for (const a of attachments) {
      if (!seen.has(a.id)) blocks.push(a.block);
    }
    return blocks.length > 0 ? blocks : null;
  }
  if (attachments.length === 0) return null;
  const blocks: ContentBlock[] = attachments.map((a) => a.block);
  const trimmed = text.trim();
  if (trimmed) blocks.push({ type: "text", text });
  return blocks;
}
