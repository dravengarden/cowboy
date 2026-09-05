import { assert, assertEquals } from "jsr:@std/assert";

const composer = await Deno.readTextFile(new URL("./Composer.tsx", import.meta.url));
const transcript = await Deno.readTextFile(new URL("./Transcript.tsx", import.meta.url));
const previewSource = await Deno.readTextFile(
  new URL("./MessagePreview.tsx", import.meta.url),
);
const inlinePreviewSource = await Deno.readTextFile(
  new URL("./mdlive/inline-preview.ts", import.meta.url),
);

Deno.test("optimistic draft cards strip image tokens instead of painting raw cowboy-att markdown", () => {
  const start = composer.indexOf("function OptimisticDraftRow(");
  const end = composer.indexOf("interface PendingEditController", start);
  assert(start >= 0 && end > start);
  const body = composer.slice(start, end);
  assert(body.includes("attachmentTrayForSurface(message.attachments, previewText)"));
  assert(body.includes("<MessagePreview"));
  assert(body.includes("attachments={message.attachments}"));
  assertEquals(body.includes("{message.text || \"📎 attachment\"}"), false);
});

Deno.test("pending rows show every delivery phase and failed rows offer return-to-home", () => {
  const start = composer.indexOf("function OptimisticDraftRow(");
  const end = composer.indexOf("interface PendingEditController", start);
  const body = composer.slice(start, end);
  assert(body.includes("CloudUpload"));
  assert(body.includes('const saving = appearance === "saving"'));
  assert(body.includes("Saving…"));
  assert(body.includes("Waiting for connection…"));
  assert(body.includes("returnFailedQueued"));
  assert(body.includes("returnLabelForHome"));
  assertEquals(
    body.includes("borderLeft: `3px solid ${t.palette.primary.main}`"),
    false,
  );
  assertEquals(
    body.includes("borderLeft: `3px solid ${t.palette.info.main}`"),
    false,
  );
});

Deno.test("tool UI selection uses fill instead of purple leading rails", () => {
  assertEquals(transcript.includes("borderLeft: 2"), false);
  assertEquals(transcript.includes("borderLeft: 3"), false);
  assertEquals(
    transcript.includes("`inset 3px 0 0 ${alpha(theme.palette.primary.main, 0.78)}`"),
    false,
  );
});

Deno.test("MessagePreview renders cowboy-att tokens as composer inline images", () => {
  assert(previewSource.includes("inlineImageField"));
  assert(previewSource.includes("seedInlineAttachments(attachments)"));
  assertEquals(previewSource.includes("compactForPreview(stripImageTokens(text))"), false);
});

Deno.test("mdlive leaves cowboy-att images for the inline widget instead of hiding them", () => {
  assert(inlinePreviewSource.includes("!imageText.includes('cowboy-att:')"));
});

Deno.test("failed transcript sends offer return to the list they left", () => {
  const start = transcript.indexOf("function OptimisticUserBubble(");
  const end = transcript.indexOf("function MessageBubble(", start);
  assert(start >= 0 && end > start);
  const body = transcript.slice(start, end);
  assert(body.includes("returnFailedMessage"));
  assert(body.includes("CloudUpload"));
  assert(body.includes('const saving = appearance === "saving"'));
  assert(body.includes("Saving…"));
  assert(body.includes("Waiting for connection…"));
});

Deno.test("local content paints and reveals before the durable transport barrier resolves", async () => {
  const store = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  const addStart = store.indexOf("async function qAdd(");
  const addEnd = store.indexOf("export function retryQueued", addStart);
  const add = store.slice(addStart, addEnd);
  assert(add.indexOf('qStatus.set(cmid, "committing")') >= 0);
  assert(add.indexOf("revealPendingArrival({") < add.indexOf("await store.mutateDurably"));
  assert(add.indexOf("await store.mutateDurably") >= 0);

  const activateStart = store.indexOf("export async function activateDraft(");
  const activateEnd = store.indexOf("export function activateAllDrafts", activateStart);
  const activate = store.slice(activateStart, activateEnd);
  assert(activate.indexOf('qStatus.set(opId, "committing")') < activate.indexOf("await qClient(sessionId).mutateDurably"));
  assert(activate.includes("row: presented"));
  assert(activate.includes("destination: dest"));
  assertEquals(activate.includes("await optimisticMessage("), false);
  assertEquals(activate.includes("await discardQueued("), false);

  const commitStart = store.indexOf("function commitQueue(");
  const commitEnd = store.indexOf("function armQTimers", commitStart);
  const commit = store.slice(commitStart, commitEnd);
  assert(commit.includes("setInteractiveState({"));
  assert(commit.includes("pendingRowStatuses"));

  const editStart = store.indexOf("async function editPendingRow(");
  const editEnd = store.indexOf("export async function requestSendQueued", editStart);
  const edit = store.slice(editStart, editEnd);
  assert(edit.indexOf('qStatus.set(opId, "committing")') < edit.indexOf("await store.mutateDurably"));
  assert(edit.includes("qStatus.delete(opId)"));

  const sendQueuedStart = store.indexOf("export async function requestSendQueued(");
  const sendQueuedEnd = store.indexOf("export async function forcePushQueued", sendQueuedStart);
  const sendQueued = store.slice(sendQueuedStart, sendQueuedEnd);
  assert(sendQueued.includes("qStatus.delete(opId)"));
  assert(sendQueued.includes("qStatus.delete(echoCmid)"));
});

Deno.test("unfocused pending draft activation actively recovers a missing server id", async () => {
  const store = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  const transport = store.slice(store.indexOf("function transmitQueueMutation("), store.indexOf("function qClient("));
  assert(transport.includes("if (command === null && isConnected())"));
  assert(transport.includes("void hydrateSession(sessionId)"));
  const hydration = store.slice(store.indexOf("async function hydrateSession("), store.indexOf("export function retrySessionHydration("));
  assert(hydration.includes("needsDraftSource(sessionId) ||"));
  assert(hydration.includes("retryableFailure = needsDraftSource(sessionId)"));
  assert(hydration.includes('qStatus.set(mutation.id, "failed")'));
  const discard = store.slice(store.indexOf("async function discardQueueMutationDurably("), store.indexOf("function pendingNamed("));
  assert(discard.includes("discardDurableDelivery(store, cmid"));
  assert(discard.includes('qStatus.set(cmid, "failed")'));
});
