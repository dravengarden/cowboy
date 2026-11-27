// Putting a sent message back in the Prompt after its turn failed.
//
// sess-1795343291454: Antigravity refused a message with an image while its
// `ask_question` waited for an answer, and the owner had to compose the
// message again. The Controller keeps no copy of a refused prompt (a
// server-side Draft restore was reviewed and withdrawn: restored attachments
// could push a session's pending snapshot past its persistence limit). The
// timeline already holds what was sent: the echoed prompt blocks, each image
// as inline bytes or a same-origin `/api/artifacts/…` URL. This module
// rebuilds the message from those blocks into the session's empty Composer,
// where the owner edits it and sends it again. Nothing is resent
// automatically, and anything it cannot rebuild exactly is reported.

import { type Attachment, nextAttachmentId } from "./attachments";
import { checkComposerAvailable, composePendingPrompt } from "./contextPendingPrompt";
import type { Draft } from "./draftStore";
import { isHumanPrompt, resolvePromptOrigin, stripLinkedSessionContext } from "./promptOrigin";
import type { ContentBlock, Envelope } from "./protocol";
import { canonicalTimeline, notify } from "./store";

/** A loaded image: base64 bytes and their MIME type. */
export interface LoadedImage {
  data: string;
  mimeType: string;
}

/** The Controller's artifact names (`valid_artifact_name`): a SHA-256 and one
 *  of its image extensions. Nothing else on this origin is read. */
const ARTIFACT_URL = /^\/api\/artifacts\/[0-9a-f]{64}\.(?:png|jpg|webp|gif|avif)$/;
const IMAGE_MIME = /^image\/[a-z0-9.+-]+$/;
const BASE64 = /^[A-Za-z0-9+/]+={0,2}$/;
/** Bounds for one restore: decoded bytes per image and in total. */
export const MAX_IMAGE_BYTES = 20 * 1024 * 1024;
export const MAX_RESTORE_BYTES = 40 * 1024 * 1024;

const decodedBytes = (base64: string): number => Math.floor((base64.length * 3) / 4);

type UserChunk = Extract<Envelope, { kind: "update" }>;

function userChunk(envelope: Envelope | undefined): UserChunk | null {
  return envelope?.kind === "update" &&
      envelope.update.sessionUpdate === "user_message_chunk"
    ? envelope
    : null;
}

/** A chunk's text as the transcript classifies it: without the linked-session
 *  wrapper Cowboy generates on a tagged first chunk. */
function chunkText(envelope: UserChunk): string {
  const content = envelope.update.content as { type?: unknown; text?: unknown } | undefined;
  return content?.type === "text" && typeof content.text === "string"
    ? stripLinkedSessionContext(content.text, envelope.cmid)
    : "";
}

function chunkOrigin(envelope: UserChunk): ReturnType<typeof resolvePromptOrigin> {
  return resolvePromptOrigin(envelope.update, chunkText(envelope));
}

/** The prompt a person sent: its echoed blocks, or why it cannot be rebuilt. */
export type SentPrompt =
  | { blocks: ContentBlock[] }
  | { unavailable: "no_prompt" | "boundary" | "history" };

/** The newest prompt a person sent before the envelope `failureSeq` (the
 *  failed turn's row), read from the canonical timeline rather than display
 *  rows. A prompt is one delivery: the run of echoed user chunks that starts
 *  at its tagged first chunk (or, untagged, the whole contiguous run), from
 *  one origin. Agent, notice and managed-call prompts are skipped. A missing
 *  failure row, or a prompt whose start may lie before the loaded history,
 *  fails closed. */
export function sentPromptBefore(timeline: readonly Envelope[], failureSeq?: number): SentPrompt {
  let end = timeline.length;
  if (failureSeq !== undefined) {
    end = timeline.findIndex((envelope) => envelope.seq === failureSeq);
    if (end < 0) return { unavailable: "boundary" };
  }
  let last = end - 1;
  while (last >= 0) {
    while (last >= 0 && !userChunk(timeline[last])) last -= 1;
    if (last < 0) break;
    let first = last;
    const actor = (index: number): string => chunkOrigin(userChunk(timeline[index])!).actor;
    const runActor = actor(last);
    while (
      first > 0 && timeline[first]?.cmid === undefined && userChunk(timeline[first - 1]) &&
      actor(first - 1) === runActor
    ) {
      first -= 1;
    }
    const run = timeline.slice(first, last + 1) as UserChunk[];
    if (isHumanPrompt(chunkOrigin(run[0]!))) {
      // The opening chunk may precede the loaded window: only a tagged first
      // chunk or the session's very first event proves the prompt is whole.
      if (first === 0 && run[0]!.cmid === undefined && run[0]!.seq > 1) {
        return { unavailable: "history" };
      }
      const blocks = run.map((envelope, index) => {
        const block = envelope.update.content as ContentBlock;
        return index === 0 && block.type === "text" && typeof block.text === "string"
          ? { ...block, text: stripLinkedSessionContext(block.text, envelope.cmid) }
          : block;
      }).filter((block) => !(block.type === "text" && block.text === ""));
      return { blocks };
    }
    last = first - 1;
  }
  return { unavailable: "no_prompt" };
}

async function readAsBase64(blob: Blob): Promise<string> {
  // Browsers encode natively; the fallback (test runtimes without FileReader)
  // keeps each argument list bounded.
  if (typeof FileReader === "undefined") {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let offset = 0; offset < bytes.length; offset += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
    }
    return btoa(binary);
  }
  return await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = (): void => {
      const url = String(reader.result);
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.onerror = (): void => reject(reader.error ?? new Error("read failed"));
    reader.readAsDataURL(blob);
  });
}

/** Read one echoed image block back into bytes: validated inline base64, or
 *  this origin's artifact store by its exact name, without redirects. The
 *  block's declared MIME type wins over the artifact's file extension. */
export async function loadPromptImage(
  block: ContentBlock,
  budget: number,
): Promise<LoadedImage | null> {
  const declared = typeof block.mimeType === "string" && IMAGE_MIME.test(block.mimeType)
    ? block.mimeType
    : undefined;
  if (typeof block.data === "string" && block.data !== "") {
    if (!declared || !BASE64.test(block.data)) return null;
    return decodedBytes(block.data) <= Math.min(budget, MAX_IMAGE_BYTES)
      ? { data: block.data, mimeType: declared }
      : null;
  }
  const url = typeof block.url === "string" ? block.url : "";
  if (!ARTIFACT_URL.test(url)) return null;
  const response = await fetch(url, {
    credentials: "same-origin",
    mode: "same-origin",
    redirect: "error",
  });
  const served = response.headers.get("content-type")?.split(";")[0]?.trim() ?? "";
  if (!response.ok || !IMAGE_MIME.test(served)) return null;
  const length = Number(response.headers.get("content-length") ?? "0");
  if (length > Math.min(budget, MAX_IMAGE_BYTES)) return null;
  const blob = await response.blob();
  if (blob.size > Math.min(budget, MAX_IMAGE_BYTES)) return null;
  return { data: await readAsBase64(blob), mimeType: declared ?? served };
}

/** A Composer draft for a sent prompt's blocks: its text exactly, and each
 *  image staged where it was as the Composer's inline token. Text that could
 *  pair with a generated token (`![` or `cowboy-att:`) keeps the images
 *  untokened, after the text, so the text itself is never consumed. Files
 *  (never kept in the timeline as bytes), unreadable images and images over
 *  the restore budget are counted as `missing`. */
export async function draftFromBlocks(
  blocks: readonly ContentBlock[],
  load: (block: ContentBlock, budget: number) => Promise<LoadedImage | null> = loadPromptImage,
): Promise<{ draft: Draft; missing: number }> {
  const texts = blocks.filter((block) => block.type === "text").map((block) => String(block.text ?? ""));
  const inline = !texts.some((text) => text.includes("![") || text.includes("cowboy-att:"));
  let text = "";
  const attachments: Attachment[] = [];
  let missing = 0;
  let budget = MAX_RESTORE_BYTES;
  for (const block of blocks) {
    if (block.type === "text") {
      text += String(block.text ?? "");
      continue;
    }
    const image = block.type === "image" ? await load(block, budget).catch(() => null) : null;
    if (!image) {
      missing += 1;
      continue;
    }
    budget -= decodedBytes(image.data);
    const id = nextAttachmentId();
    attachments.push({
      id,
      name: "image",
      mimeType: image.mimeType,
      isImage: true,
      previewUrl: `data:${image.mimeType};base64,${image.data}`,
      block: { type: "image", data: image.data, mimeType: image.mimeType },
    });
    if (inline) text += `![image](cowboy-att:${id})`;
  }
  return { draft: { text, attachments }, missing };
}

const UNAVAILABLE = {
  no_prompt: "There is no message you sent to put back.",
  boundary: "That failed turn is no longer loaded; scroll to it and try again.",
  history: "Load earlier history first: the start of that message is not loaded.",
} as const;

/** Put the newest prompt a person sent before the failed turn's row
 *  (`failureKey`, its timeline sequence) back in the session's empty Composer
 *  and focus it. */
export async function putSentMessageBack(sessionId: string, failureKey?: string): Promise<void> {
  const sent = sentPromptBefore(
    canonicalTimeline(sessionId) ?? [],
    failureKey === undefined ? undefined : Number(failureKey),
  );
  if ("unavailable" in sent) {
    notify(UNAVAILABLE[sent.unavailable]);
    return;
  }
  try {
    checkComposerAvailable(sessionId);
    const { draft, missing } = await draftFromBlocks(sent.blocks);
    if (draft.text.trim() === "" && draft.attachments.length === 0) {
      notify("Nothing in that message could be restored; attach it again.");
      return;
    }
    composePendingPrompt(sessionId, draft);
    notify(
      missing > 0
        ? `Message is back in the Prompt; ${missing} attachment${missing === 1 ? "" : "s"} ` +
          "could not be restored. Attach again before sending."
        : "Message is back in the Prompt.",
      missing > 0 ? "warning" : "info",
    );
  } catch (error) {
    notify(error instanceof Error ? error.message : "Could not put the message back");
  }
}
