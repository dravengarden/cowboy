import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import { imageTokensInText } from "./attachments.ts";
import type { RenderItem } from "./derive.ts";
import {
  draftFromMessage,
  lastSentMessage,
  loadTranscriptImage,
} from "./refusedMessage.ts";
import { readTranscriptSource } from "./appSource.test.ts";

const transcript = await readTranscriptSource();

const user = (
  key: string,
  text: string,
  actor: "human" | "agent" = "human",
): RenderItem => ({
  key,
  kind: "message",
  role: "user",
  chunks: [{ type: "text", text }],
  origin: { actor, source: actor === "human" ? "composer" : "runtime" },
});

test("the message put back is the newest one a person sent before the failure", () => {
  const failure: RenderItem = {
    key: "9",
    kind: "lifecycle",
    status: "crashed",
    detail: "x",
    turnFailure: true,
  };
  const items: RenderItem[] = [
    user("1", "older"),
    user("5", "refused"),
    user("7", "notice", "agent"),
    failure,
    user("12", "later answer"),
  ];
  assertEquals(lastSentMessage(items, "9")?.key, "5");
  assertEquals(lastSentMessage(items)?.key, "12");
  // Agent-originated prompts are never offered back as the owner's.
  assertEquals(lastSentMessage([user("3", "notice", "agent")]), null);
});

test("a sent message becomes a Composer draft with each image in place", async () => {
  // sess-1795343291454: an image and its request, refused while Antigravity
  // awaited an answer.
  const { draft, missing } = await draftFromMessage(
    [
      { type: "image", src: "data:image/png;base64,aW1n" },
      { type: "text", text: "\n \n还有 dialog 要尽可能利用屏幕空间" },
      { type: "image", src: "/api/artifacts/b.png" },
      { type: "file", name: "notes.txt" },
      { type: "image", src: "/api/artifacts/gone.png" },
    ],
    (src) =>
      Promise.resolve(
        src === "data:image/png;base64,aW1n"
          ? { data: "aW1n", mimeType: "image/png" }
          : src === "/api/artifacts/b.png"
          ? { data: "Yg==", mimeType: "image/jpeg" }
          : null,
      ),
  );
  // A file's bytes never reach the transcript, and a gone image cannot be
  // read: both are reported, not silently dropped.
  assertEquals(missing, 2);
  assertEquals(draft.attachments.length, 2);
  const tokens = imageTokensInText(draft.text);
  assertEquals(tokens.map((token) => token.id), draft.attachments.map((a) => a.id));
  assert(draft.text.startsWith("![image](cowboy-att:"));
  assert(draft.text.includes(")\n \n还有 dialog 要尽可能利用屏幕空间![image](cowboy-att:"));
  assertEquals(draft.attachments[0]?.block, {
    type: "image",
    data: "aW1n",
    mimeType: "image/png",
  });
  assertEquals(draft.attachments[1]?.mimeType, "image/jpeg");
  assert(draft.attachments.every((attachment) => attachment.isImage && !attachment.pending));
});

test("only inline images and this origin's artifacts are read back", async () => {
  assertEquals(await loadTranscriptImage("data:image/gif;base64,R0lG"), {
    mimeType: "image/gif",
    data: "R0lG",
  });
  assertEquals(await loadTranscriptImage("https://example.com/a.png"), null);
  const original = globalThis.fetch;
  const requested: string[] = [];
  globalThis.fetch = ((input: RequestInfo | URL): Promise<Response> => {
    requested.push(String(input));
    return Promise.resolve(
      String(input).endsWith("missing.png")
        ? new Response("", { status: 404 })
        : new Response(new Blob([new Uint8Array([105, 109, 103])], { type: "image/webp" })),
    );
  }) as typeof fetch;
  try {
    assertEquals(await loadTranscriptImage("/api/artifacts/a.webp"), {
      mimeType: "image/webp",
      data: "aW1n",
    });
    assertEquals(await loadTranscriptImage("/api/artifacts/missing.png"), null);
  } finally {
    globalThis.fetch = original;
  }
  assertEquals(requested, ["/api/artifacts/a.webp", "/api/artifacts/missing.png"]);
});

test("every failed turn offers Back to Prompt, and refused input leads with it", () => {
  assert(
    transcript.includes(
      "<PutBackButton sessionId={sessionId} beforeKey={itemKey} primary={refusedInput} />",
    ),
  );
  // A remote refusal shows `crashed`, which hides the failed turn's row behind
  // the status bar; the bar carries the action then.
  assert(transcript.includes("<PutBackButton sessionId={sessionId} primary />"));
});
