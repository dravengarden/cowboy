// Putting a sent message back in the Prompt after its turn failed.
//
// sess-1795343291454: Antigravity refused a message with an image while its
// `ask_question` waited for an answer, and the owner had to compose the
// message again. The Controller keeps no copy of a refused prompt (a
// server-side Draft restore was reviewed and withdrawn: restored attachments
// could push a session's pending snapshot past its persistence limit). The
// transcript already holds what was sent: its text, and each image as inline
// bytes or a same-origin `/api/artifacts/…` URL. This module rebuilds the
// message from there into the session's empty Composer, where the owner edits
// it and sends it again. Nothing is resent automatically.

import { type Attachment, nextAttachmentId } from "./attachments";
import { checkComposerAvailable, composePendingPrompt } from "./contextPendingPrompt";
import { type ContentChunk, derive, type RenderItem } from "./derive";
import type { Draft } from "./draftStore";
import { isHumanPrompt } from "./promptOrigin";
import { canonicalTimeline, notify } from "./store";

type UserMessage = Extract<RenderItem, { kind: "message" }>;

/** A loaded image: base64 bytes and their MIME type. */
export interface LoadedImage {
  data: string;
  mimeType: string;
}

/** Where a transcript image may be read from: inline bytes, or this origin's
 *  artifact store (the Controller externalizes large prompt images there). */
const ARTIFACT_PATH = "/api/artifacts/";

/** The newest message a person sent, at or before `beforeKey` (the failed
 *  turn's row) when given. Agent-originated prompts (notices, managed calls,
 *  peers) are never offered back as the owner's. */
export function lastSentMessage(
  items: readonly RenderItem[],
  beforeKey?: string,
): UserMessage | null {
  let end = items.length;
  if (beforeKey !== undefined) {
    const at = items.findIndex((item) => item.key === beforeKey);
    if (at >= 0) end = at;
  }
  for (let index = end - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item?.kind === "message" && item.role === "user" && isHumanPrompt(item.origin)) {
      return item;
    }
  }
  return null;
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  // Bounded argument lists: a multi-megabyte screenshot in one call would
  // overflow the engine's argument limit.
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary);
}

/** Read one transcript image back into bytes. Only inline `data:` images and
 *  this origin's artifact URLs qualify; anything else is not restorable. */
export async function loadTranscriptImage(src: string): Promise<LoadedImage | null> {
  const inline = /^data:([^;,]+);base64,(.+)$/s.exec(src);
  if (inline) return { mimeType: inline[1] ?? "image/png", data: inline[2] ?? "" };
  if (!src.startsWith(ARTIFACT_PATH)) return null;
  const response = await fetch(src, { credentials: "same-origin" });
  if (!response.ok) return null;
  const blob = await response.blob();
  const mimeType = blob.type.startsWith("image/") ? blob.type : "image/png";
  return { mimeType, data: bytesToBase64(new Uint8Array(await blob.arrayBuffer())) };
}

/** A Composer draft for a sent message: its text in order, each image where
 *  it was as the Composer's inline token with a staged attachment. Files
 *  (whose bytes the transcript never holds) and images that cannot be read
 *  are counted as `missing`, never silently dropped from the report. */
export async function draftFromMessage(
  chunks: readonly ContentChunk[],
  load: (src: string) => Promise<LoadedImage | null> = loadTranscriptImage,
): Promise<{ draft: Draft; missing: number }> {
  let text = "";
  const attachments: Attachment[] = [];
  let missing = 0;
  for (const chunk of chunks) {
    if (chunk.type === "text") {
      text += chunk.text;
      continue;
    }
    if (chunk.type === "file") {
      missing += 1;
      continue;
    }
    const image = await load(chunk.src).catch(() => null);
    if (!image?.data) {
      missing += 1;
      continue;
    }
    const id = nextAttachmentId();
    attachments.push({
      id,
      name: "image",
      mimeType: image.mimeType,
      isImage: true,
      previewUrl: `data:${image.mimeType};base64,${image.data}`,
      block: { type: "image", data: image.data, mimeType: image.mimeType },
    });
    text += `![image](cowboy-att:${id})`;
  }
  return { draft: { text, attachments }, missing };
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : "Could not put the message back";
}

/** Put the session's newest sent message (before `beforeKey`, the failed
 *  turn's row, when given) back in its empty Composer and focus it. */
export async function putSentMessageBack(sessionId: string, beforeKey?: string): Promise<void> {
  const sent = lastSentMessage(derive(canonicalTimeline(sessionId) ?? []), beforeKey);
  if (!sent) {
    notify("There is no sent message to put back.");
    return;
  }
  try {
    checkComposerAvailable(sessionId);
    const { draft, missing } = await draftFromMessage(sent.chunks);
    composePendingPrompt(sessionId, draft);
    if (missing > 0) {
      notify(
        `Message is back in the Prompt; ${missing} attachment${missing === 1 ? "" : "s"} ` +
          "could not be restored. Attach again before sending.",
        "warning",
      );
    } else {
      notify("Message is back in the Prompt.", "info");
    }
  } catch (error) {
    notify(message(error));
  }
}
