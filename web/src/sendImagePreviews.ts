import { type Attachment, isLoadablePreviewUrl } from "./attachments";
import type { Envelope } from "./protocol";

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
  if (needed === 0) return true;
  let images = 0;
  for (let index = start; index < timeline.length; index += 1) {
    const type = userMessageChunkType(timeline[index]!);
    if (type === undefined) break;
    if (type === "image") images += 1;
  }
  return images >= needed;
}
