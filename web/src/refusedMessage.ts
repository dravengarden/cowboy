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
/** Standard base64 by a linear scan: a repeated regex group would exhaust
 *  V8's stack on a multi-megabyte image. */
function validBase64(data: string): boolean {
  if (data.length === 0 || data.length % 4 !== 0) return false;
  const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
  for (let index = 0; index < data.length - padding; index += 1) {
    const code = data.charCodeAt(index);
    const ok = (code >= 65 && code <= 90) || (code >= 97 && code <= 122) ||
      (code >= 48 && code <= 57) || code === 43 || code === 47;
    if (!ok) return false;
  }
  return true;
}
/** Bounds for one restore: decoded bytes per image and in total. */
export const MAX_IMAGE_BYTES = 20 * 1024 * 1024;
export const MAX_RESTORE_BYTES = 40 * 1024 * 1024;

const decodedBytes = (base64: string): number =>
  (base64.length / 4) * 3 - (base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0);

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
  | { unavailable: "no_prompt" | "boundary" | "history" | "unidentified" };

interface Run {
  first: number;
  tagged: boolean;
  human: boolean;
  blocks: ContentBlock[];
}

/** The run of echoed user chunks ending at `last`: back to its tagged
 *  opening chunk (the delivery the Controller dispatched), or over the
 *  untagged contiguous chunks of one origin. */
function runEndingAt(timeline: readonly Envelope[], last: number): Run {
  const actor = (index: number): string => chunkOrigin(userChunk(timeline[index])!).actor;
  let first = last;
  while (
    first > 0 && timeline[first]?.cmid === undefined && userChunk(timeline[first - 1]) &&
    actor(first - 1) === actor(last)
  ) {
    first -= 1;
  }
  const run = timeline.slice(first, last + 1) as UserChunk[];
  const blocks = run.map((envelope, index) => {
    const block = envelope.update.content as ContentBlock;
    return index === 0 && block.type === "text" && typeof block.text === "string"
      ? { ...block, text: stripLinkedSessionContext(block.text, envelope.cmid) }
      : block;
  }).filter((block) => !(block.type === "text" && block.text === ""));
  return {
    first,
    tagged: run[0]!.cmid !== undefined,
    human: isHumanPrompt(chunkOrigin(run[0]!)),
    blocks,
  };
}

const same = (left: readonly ContentBlock[], right: readonly ContentBlock[]): boolean =>
  JSON.stringify(left) === JSON.stringify(right);

/** Whether a delivery ends by repeating its own opening blocks: an adjacent
 *  legacy accept echo, or a person who sent the same attachment twice. The
 *  record cannot tell them apart, so such a delivery is not rebuilt. */
function endsWithRepeat(blocks: readonly ContentBlock[]): boolean {
  for (let split = Math.ceil(blocks.length / 2); split < blocks.length; split += 1) {
    if (same(blocks.slice(split), blocks.slice(0, blocks.length - split))) return true;
  }
  return false;
}

/** The newest prompt a person sent before the envelope `failureSeq` (the
 *  failed turn's row), read from the canonical timeline rather than display
 *  rows. Only a delivery identified by its tagged opening chunk is rebuilt;
 *  an untagged run counts only as a replay of the tagged delivery before it
 *  (anything else is `unidentified`). Agent, notice and managed-call prompts
 *  are skipped. `coveredFrom` is the first sequence the loaded history holds
 *  densely (older events are retained markers only), or null when it reaches
 *  the session's start: a delivery starting before it is `history`. A
 *  missing failure row fails closed. */
export function sentPromptBefore(
  timeline: readonly Envelope[],
  failureSeq?: number,
  coveredFrom: number | null = null,
): SentPrompt {
  let last = timeline.length - 1;
  if (failureSeq !== undefined) {
    last = timeline.findIndex((envelope) => envelope.seq === failureSeq) - 1;
    if (last < -1) return { unavailable: "boundary" };
  }
  let replay: ContentBlock[] | null = null;
  while (last >= 0) {
    if (!userChunk(timeline[last])) {
      last -= 1;
      continue;
    }
    const run = runEndingAt(timeline, last);
    last = run.first - 1;
    if (!run.human) continue;
    if (!run.tagged) {
      // Untagged after a replay already seen: no identified delivery left.
      if (replay !== null) return { unavailable: "unidentified" };
      replay = run.blocks;
      continue;
    }
    if (coveredFrom !== null && timeline[run.first]!.seq < coveredFrom) {
      return { unavailable: "history" };
    }
    const blocks = run.blocks;
    // A literal attachment marker in the text would be parsed as a token and
    // dropped on resend: the prompt cannot round-trip, so it is not rebuilt.
    const literal = blocks.some((block) =>
      block.type === "text" && String(block.text ?? "").includes("cowboy-att:")
    );
    if (literal || endsWithRepeat(blocks)) return { unavailable: "unidentified" };
    // A separated untagged run must repeat this delivery, or it was a prompt
    // whose identity was not kept: never substitute an older delivery.
    if (replay !== null && !same(replay, blocks.slice(0, replay.length))) {
      return { unavailable: "unidentified" };
    }
    return { blocks };
  }
  return { unavailable: replay === null ? "no_prompt" : "unidentified" };
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
    // Size first: an image over the cap is never scanned.
    if (!declared || block.data.length > Math.ceil(Math.min(budget, MAX_IMAGE_BYTES) / 3) * 4) {
      return null;
    }
    if (!validBase64(block.data) || decodedBytes(block.data) === 0) return null;
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
  const limit = Math.min(budget, MAX_IMAGE_BYTES);
  if (Number(response.headers.get("content-length") ?? "0") > limit) return null;
  // Read under the cap: a response without a usable length never buffers
  // more than the budget allows.
  const parts: Uint8Array<ArrayBuffer>[] = [];
  let size = 0;
  const reader = response.body?.getReader();
  if (!reader) return null;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > limit) {
      await reader.cancel();
      return null;
    }
    parts.push(new Uint8Array(value));
  }
  if (size === 0) return null;
  return { data: await readAsBase64(new Blob(parts)), mimeType: declared ?? served };
}

/** A Composer draft for a sent prompt's blocks: its text exactly, and each
 *  image staged where it was as the Composer's inline token. Files
 *  (never kept in the timeline as bytes), unreadable images and images over
 *  the restore budget are counted as `missing`. */
export async function draftFromBlocks(
  blocks: readonly ContentBlock[],
  load: (block: ContentBlock, budget: number) => Promise<LoadedImage | null> = loadPromptImage,
): Promise<{ draft: Draft; missing: number }> {
  const texts = blocks.filter((block) => block.type === "text").map((block) => String(block.text ?? ""));
  // The Composer pairs inline tokens by Markdown image syntax: text that holds
  // its own (`![`) could absorb a generated token and drop text on send, so
  // such a prompt returns as text and its images are reported for attaching
  // again. (A literal `cowboy-att:` marker is refused before this point.)
  const ambiguous = texts.some((text) => text.includes("!["));
  let text = "";
  const attachments: Attachment[] = [];
  let missing = 0;
  let budget = MAX_RESTORE_BYTES;
  for (const block of blocks) {
    if (block.type === "text") {
      text += String(block.text ?? "");
      continue;
    }
    const image = block.type === "image" && !ambiguous
      ? await load(block, budget).catch(() => null)
      : null;
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
    text += `![image](cowboy-att:${id})`;
  }
  return { draft: { text, attachments }, missing };
}

const UNAVAILABLE = {
  no_prompt: "There is no message you sent to put back.",
  boundary: "That failed turn is no longer loaded; scroll to it and try again.",
  history: "Load earlier history first: the start of that message is not loaded.",
  unidentified: "That message cannot be rebuilt exactly; compose it again.",
} as const;

/** Put the newest prompt a person sent before the failed turn's row
 *  (`failureKey`, its timeline sequence) back in the session's empty Composer
 *  and focus it. `coveredFrom` is the loaded history's dense start. */
export async function putSentMessageBack(
  sessionId: string,
  failureKey: string,
  coveredFrom: number | null,
): Promise<void> {
  const select = (): SentPrompt =>
    sentPromptBefore(canonicalTimeline(sessionId) ?? [], Number(failureKey), coveredFrom);
  const sent = select();
  if ("unavailable" in sent) {
    notify(UNAVAILABLE[sent.unavailable]);
    return;
  }
  try {
    checkComposerAvailable(sessionId);
    const { draft, missing } = await draftFromBlocks(sent.blocks);
    // Images load asynchronously: a Clear or a reset meanwhile removes the
    // failure, and its prompt must not reach the new conversation.
    const now = select();
    if ("unavailable" in now || !same(now.blocks, sent.blocks)) {
      notify("The conversation changed; nothing was put back.");
      return;
    }
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
