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

let attachSeq = 0;
function nextId(): string {
  attachSeq += 1;
  return `att${attachSeq}`;
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

/// Convert one picked / pasted file into a stageable `Attachment`. Throws (via a
/// rejected promise) only on a FileReader error — callers should `.catch` and
/// drop that single file rather than failing the whole batch.
export async function fileToAttachment(file: File): Promise<Attachment> {
  const mimeType = file.type || "application/octet-stream";
  if (mimeType.startsWith("image/")) {
    const data = await readBase64(file);
    return {
      id: nextId(),
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
      id: nextId(),
      name: file.name || "attachment.txt",
      mimeType,
      isImage: false,
      block: { type: "resource", resource: { uri, text, mimeType } },
    };
  }
  const blob = await readBase64(file);
  return {
    id: nextId(),
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

/// Build the ACP `content` block array for a prompt: the attachment blocks
/// first (so the agent has the visual / file context before the instruction),
/// then a trailing text block when there's any text. Returns null when there's
/// nothing to attach — callers then use the plain text-only prompt path.
export function buildContentBlocks(
  text: string,
  attachments: readonly Attachment[],
): ContentBlock[] | null {
  if (attachments.length === 0) return null;
  const blocks: ContentBlock[] = attachments.map((a) => a.block);
  const trimmed = text.trim();
  if (trimmed) blocks.push({ type: "text", text });
  return blocks;
}
