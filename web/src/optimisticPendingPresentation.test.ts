import { assert, assertEquals } from "jsr:@std/assert";

const composer = await Deno.readTextFile(new URL("./Composer.tsx", import.meta.url));
const transcript = await Deno.readTextFile(new URL("./Transcript.tsx", import.meta.url));

Deno.test("optimistic draft cards strip image tokens instead of painting raw cowboy-att markdown", () => {
  const start = composer.indexOf("function OptimisticDraftRow(");
  const end = composer.indexOf("interface PendingEditController", start);
  assert(start >= 0 && end > start);
  const body = composer.slice(start, end);
  assert(body.includes("stripImageTokens(message.text)"));
  assert(body.includes("<MessagePreview text={preview} />"));
  assertEquals(body.includes("{message.text || \"📎 attachment\"}"), false);
});

Deno.test("unsynced rows show an uploading mark and failed rows offer return-to-home", () => {
  const start = composer.indexOf("function OptimisticDraftRow(");
  const end = composer.indexOf("interface PendingEditController", start);
  const body = composer.slice(start, end);
  assert(body.includes("CloudUpload"));
  assert(body.includes("Waiting to sync"));
  assert(body.includes("returnFailedQueued"));
  assert(body.includes("returnLabelForHome"));
});

Deno.test("failed transcript sends offer return to the list they left", () => {
  const start = transcript.indexOf("function OptimisticUserBubble(");
  const end = transcript.indexOf("function MessageBubble(", start);
  assert(start >= 0 && end > start);
  const body = transcript.slice(start, end);
  assert(body.includes("returnFailedMessage"));
  assert(body.includes("CloudUpload"));
  assert(body.includes("Waiting to sync"));
});
