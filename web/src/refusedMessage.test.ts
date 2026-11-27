import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import { imageTokensInText } from "./attachments.ts";
import type { ContentBlock, Envelope } from "./protocol.ts";
import {
  draftFromBlocks,
  loadPromptImage,
  MAX_IMAGE_BYTES,
  sentPromptBefore,
} from "./refusedMessage.ts";
import { readTranscriptSource } from "./appSource.test.ts";

const transcript = await readTranscriptSource();
const ARTIFACT = `/api/artifacts/${"b".repeat(64)}.png`;

const chunk = (
  seq: number,
  content: ContentBlock,
  extra: { cmid?: string; actor?: "human" | "agent" } = {},
): Envelope => ({
  session_id: "s",
  seq,
  ...(extra.cmid ? { cmid: extra.cmid } : {}),
  kind: "update",
  update: {
    sessionUpdate: "user_message_chunk",
    content,
    ...(extra.actor
      ? {
        promptOrigin: {
          actor: extra.actor,
          source: extra.actor === "human" ? "composer" : "runtime",
        },
      }
      : {}),
  },
});
const agent = (seq: number, text: string): Envelope => ({
  session_id: "s",
  seq,
  kind: "update",
  update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text } },
});
const failure = (seq: number): Envelope => ({
  session_id: "s",
  seq,
  kind: "lifecycle",
  status: "crashed",
  detail: "antigravity_prompt_rejected:image_answer: {}",
});

test("the prompt put back is the newest one a person sent before the failure", () => {
  const image: ContentBlock = { type: "image", mimeType: "image/png", url: ARTIFACT };
  const timeline: Envelope[] = [
    chunk(1, { type: "text", text: "first" }, { cmid: "a" }),
    agent(2, "a question?"),
    // The refused prompt: indentation and the tagged opening chunk survive.
    chunk(3, { type: "text", text: "  indented\n    code" }, { cmid: "b" }),
    chunk(4, image),
    chunk(5, { type: "text", text: "tail" }),
    // A notice injected later is never the owner's.
    chunk(6, { type: "text", text: "notice" }, { actor: "agent" }),
    failure(7),
    chunk(8, { type: "text", text: "the text answer" }, { cmid: "c" }),
  ];
  assertEquals(sentPromptBefore(timeline, 7), {
    blocks: [{ type: "text", text: "  indented\n    code" }, image, { type: "text", text: "tail" }],
  });
  // Without a boundary the newest prompt counts; a missing boundary fails
  // closed rather than picking a newer message.
  assertEquals(sentPromptBefore(timeline), { blocks: [{ type: "text", text: "the text answer" }] });
  assertEquals(sentPromptBefore(timeline, 99), { unavailable: "boundary" });
  // A prompt whose opening may precede the loaded window is not guessed at.
  assertEquals(sentPromptBefore(timeline.slice(3), 7), { unavailable: "history" });
  assertEquals(sentPromptBefore([agent(1, "x"), failure(2)], 2), { unavailable: "no_prompt" });
});

test("the generated linked-session wrapper leaves the tagged first block", () => {
  const wrapped = "<system-reminder>\nLinked Cowboy sessions: a\n</system-reminder>\n\nmy prompt";
  assertEquals(
    sentPromptBefore([chunk(1, { type: "text", text: wrapped }, { cmid: "x" }), failure(2)], 2),
    { blocks: [{ type: "text", text: "my prompt" }] },
  );
});

test("a sent prompt becomes a Composer draft with each image in place", async () => {
  const blocks: ContentBlock[] = [
    { type: "image", mimeType: "image/png", data: "aW1n" },
    { type: "text", text: "\n \n还有 dialog  要尽可能利用屏幕空间" },
    { type: "image", mimeType: "image/jpeg", url: ARTIFACT },
    { type: "resource", resource: { uri: "file:///notes.txt", text: "x" } },
    { type: "image", mimeType: "image/png", url: `/api/artifacts/${"c".repeat(64)}.png` },
  ];
  const { draft, missing } = await draftFromBlocks(blocks, (block) =>
    Promise.resolve(
      block.data === "aW1n"
        ? { data: "aW1n", mimeType: "image/png" }
        : block.url === ARTIFACT
        ? { data: "Yg==", mimeType: "image/jpeg" }
        : null,
    ));
  // A file's bytes never reach the timeline, and a gone image cannot be read:
  // both are reported, not silently dropped.
  assertEquals(missing, 2);
  assertEquals(draft.attachments.length, 2);
  assertEquals(
    imageTokensInText(draft.text).map((token) => token.id),
    draft.attachments.map((attachment) => attachment.id),
  );
  assert(draft.text.includes(")\n \n还有 dialog  要尽可能利用屏幕空间![image](cowboy-att:"));
  assertEquals(draft.attachments[0]?.block, { type: "image", data: "aW1n", mimeType: "image/png" });
  assertEquals(draft.attachments[1]?.mimeType, "image/jpeg");
  // Text that could pair with a generated token keeps the images untokened.
  const quoted = await draftFromBlocks(
    [{ type: "text", text: "see ![unfinished" }, { type: "image", mimeType: "image/png", data: "aW1n" }],
    () => Promise.resolve({ data: "aW1n", mimeType: "image/png" }),
  );
  assertEquals(quoted.draft.text, "see ![unfinished");
  assertEquals(quoted.draft.attachments.length, 1);
});

test("only validated inline images and exact artifact names are read back", async () => {
  const budget = MAX_IMAGE_BYTES;
  assertEquals(
    await loadPromptImage({ type: "image", mimeType: "image/gif", data: "R0lG" }, budget),
    { mimeType: "image/gif", data: "R0lG" },
  );
  for (
    const block of [
      { type: "image", mimeType: "text/html", data: "R0lG" },
      { type: "image", mimeType: "image/png", data: "not base64!" },
      { type: "image", mimeType: "image/png", url: "https://example.com/a.png" },
      { type: "image", mimeType: "image/png", url: "/api/artifacts/../sessions" },
      { type: "image", mimeType: "image/png", url: `/api/artifacts/${"b".repeat(64)}.svg` },
    ]
  ) {
    assertEquals(await loadPromptImage(block, budget), null, JSON.stringify(block));
  }
  const original = globalThis.fetch;
  const requested: { url: string; init: RequestInit | undefined }[] = [];
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    requested.push({ url: String(input), init });
    return Promise.resolve(
      String(input).includes("d".repeat(64))
        ? new Response("<html>", { headers: { "content-type": "text/html" } })
        : new Response(new Blob([new Uint8Array([105, 109, 103])]), {
          headers: { "content-type": "image/webp" },
        }),
    );
  }) as typeof fetch;
  try {
    // The block's declared type wins over the stored file's.
    assertEquals(
      await loadPromptImage({ type: "image", mimeType: "image/avif", url: ARTIFACT }, budget),
      { mimeType: "image/avif", data: "aW1n" },
    );
    // A response that is not an image is never staged as one.
    assertEquals(
      await loadPromptImage(
        { type: "image", mimeType: "image/png", url: `/api/artifacts/${"d".repeat(64)}.png` },
        budget,
      ),
      null,
    );
    // Over the remaining restore budget: not read into the draft.
    assertEquals(await loadPromptImage({ type: "image", url: ARTIFACT }, 2), null);
  } finally {
    globalThis.fetch = original;
  }
  assertEquals(requested[0]?.init?.redirect, "error");
  assertEquals(requested[0]?.init?.mode, "same-origin");
});

test("every failed turn offers Back to Prompt, keyed to its own failure", () => {
  assert(
    transcript.includes(
      "<PutBackButton sessionId={sessionId} failureKey={itemKey} primary={refusedInput} />",
    ),
  );
  // A remote refusal shows `crashed`, which hides the failed turn's row behind
  // the status bar; the bar carries the action, keyed to that crash.
  assert(transcript.includes("<PutBackButton sessionId={sessionId} failureKey={crashKey} primary />"));
});
