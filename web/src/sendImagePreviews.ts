import { type Attachment, isLoadablePreviewUrl } from "./attachments";
import type { Envelope } from "./protocol";
import { isTurnActivityUpdate } from "./turnWaiting";

const previewsByCmid = new Map<string, string[]>();

/** Keep the in-memory send preview so the confirmed bubble can paint the same
 * pixels instead of remounting an `/api/artifacts/…` fetch. */
export function rememberSendImagePreviews(
  cmid: string,
  attachments: readonly Attachment[],
): void {
  const urls = attachments.flatMap((attachment) =>
    attachment.isImage && isLoadablePreviewUrl(attachment.previewUrl)
      ? [attachment.previewUrl]
      : []
  );
  if (urls.length > 0) previewsByCmid.set(cmid, urls);
}

export function sendImagePreview(
  cmid: string | undefined,
  imageIndex: number,
): string | undefined {
  if (cmid === undefined || imageIndex < 0) return undefined;
  const url = previewsByCmid.get(cmid)?.[imageIndex];
  return isLoadablePreviewUrl(url) ? url : undefined;
}

export function confirmedImageSrc(
  chunkSrc: string,
  cmid: string | undefined,
  imageIndex: number,
): string {
  return sendImagePreview(cmid, imageIndex) ?? chunkSrc;
}

export function applySendImagePreviews<
  T extends { type: string; src?: string },
>(chunks: readonly T[], cmid: string | undefined): readonly T[] {
  if (cmid === undefined) return chunks;
  let imageIndex = 0;
  let changed = false;
  const next = chunks.map((chunk) => {
    if (chunk.type !== "image" || typeof chunk.src !== "string") return chunk;
    const src = confirmedImageSrc(chunk.src, cmid, imageIndex);
    imageIndex += 1;
    if (src === chunk.src) return chunk;
    changed = true;
    return { ...chunk, src };
  });
  return changed ? next : chunks;
}

function userMessageChunkType(env: Envelope): string | undefined {
  if (env.kind !== "update") return undefined;
  if (env.update.sessionUpdate !== "user_message_chunk") return undefined;
  const content = env.update.content as { type?: unknown } | undefined;
  return typeof content?.type === "string" ? content.type : undefined;
}

function userMessageChunkContent(
  env: Envelope,
): {
  type?: string;
  data?: unknown;
  url?: unknown;
  source?: { data?: unknown; url?: unknown };
} | undefined {
  if (env.kind !== "update") return undefined;
  if (env.update.sessionUpdate !== "user_message_chunk") return undefined;
  return env.update.content as {
    type?: string;
    data?: unknown;
    url?: unknown;
    source?: { data?: unknown; url?: unknown };
  } | undefined;
}

function hasRenderableMediaSource(
  source: { data?: unknown; url?: unknown } | undefined,
): boolean {
  return typeof source?.url === "string" && source.url.length > 0 ||
    typeof source?.data === "string" && source.data.length > 0;
}

/** A tagged echo is not the same as a painted bubble. Empty image blocks and
 * unknown content types exist on the wire before `derive` can mount a row. */
export function isRenderableUserMessageChunk(env: Envelope): boolean {
  const content = userMessageChunkContent(env);
  const type = content?.type;
  if (type === "text" || type === "resource" || type === "resource_link") {
    return true;
  }
  if (type !== "image") return false;
  return hasRenderableMediaSource(content) ||
    hasRenderableMediaSource(content?.source);
}

/** The first echo carries `cmid`; later blocks of the same prompt are untagged.
 * Dropping the optimistic bubble on that first envelope leaves a text-only or
 * artifact-URL row until the rest arrives, which is the flash the user sees. */
export function promptEchoReadyToReplaceOptimistic(
  message: {
    cmid?: string;
    attachments?: readonly { isImage?: boolean }[];
  },
  timeline: readonly Envelope[],
): boolean {
  const cmid = message.cmid;
  if (cmid === undefined) return false;
  const start = timeline.findIndex((env) => env.cmid === cmid);
  if (start < 0) return false;
  const needed = (message.attachments ?? []).filter((attachment) =>
    attachment.isImage
  ).length;
  let images = 0;
  let renderable = false;
  for (let index = start; index < timeline.length; index += 1) {
    const env = timeline[index]!;
    const type = userMessageChunkType(env);
    if (type === undefined) break;
    if (isRenderableUserMessageChunk(env)) renderable = true;
    if (type === "image" && isRenderableUserMessageChunk(env)) images += 1;
  }
  return renderable && images >= needed;
}

/** The daemon echoes every block of a prompt before the provider starts that
 * turn, so agent work after a tagged echo proves the whole prompt has already
 * been echoed. Lifecycle frames can interleave between echo blocks and do not
 * count. */
export function envelopeCompletesPromptEcho(env: Envelope): boolean {
  if (env.kind === "turn_end" || env.kind === "permission_request") return true;
  return env.kind === "update" &&
    env.update.sessionUpdate !== "user_message_chunk" &&
    isTurnActivityUpdate(env.update.sessionUpdate);
}

/** Keep a local send visible until the *presented* timeline can replace it.
 * Canonical echo can drop the store overlay while Transcript is still frozen
 * for a swipe/scroll, which is the disappear-then-reappear hole. Once the
 * renderer presents the canonical timeline again the store is authoritative:
 * a copy kept past that point hides the newest human row and repaints an
 * already-answered prompt at the bottom of the transcript. */
export function retainUnpresentedOptimistic<
  T extends {
    cmid?: string;
    attachments?: readonly { isImage?: boolean }[];
  },
>(
  previous: readonly T[],
  fromStore: readonly T[],
  presentedTimeline: readonly Envelope[],
  presentationFrozen: boolean,
): readonly T[] {
  if (!presentationFrozen) return fromStore;
  const fromStoreIds = new Set(
    fromStore.flatMap((message) =>
      message.cmid === undefined ? [] : [message.cmid]
    ),
  );
  const lingering = previous.filter((message) =>
    message.cmid !== undefined &&
    !fromStoreIds.has(message.cmid) &&
    !promptEchoReadyToReplaceOptimistic(message, presentedTimeline)
  );
  return lingering.length === 0 ? fromStore : [...fromStore, ...lingering];
}
