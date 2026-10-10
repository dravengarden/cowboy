import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const composer = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);
const transcript = await readFile(
  new URL("./Transcript.tsx", import.meta.url), "utf8",
);
const previewSource = await readFile(
  new URL("./MessagePreview.tsx", import.meta.url), "utf8",
);
const inlinePreviewSource = await readFile(
  new URL("./mdlive/inline-preview.ts", import.meta.url), "utf8",
);

test("optimistic draft cards strip image tokens instead of painting raw cowboy-att markdown", () => {
  const start = composer.indexOf("function OptimisticDraftRow(");
  const end = composer.indexOf("interface PendingEditController", start);
  assert(start >= 0 && end > start);
  const body = composer.slice(start, end);
  assert(
    body.includes("attachmentTrayForSurface(message.attachments, previewText)"),
  );
  assert(body.includes("<MessagePreview"));
  assert(body.includes("attachments={message.attachments}"));
  assertEquals(body.includes('{message.text || "📎 attachment"}'), false);
});

test("pending rows show every delivery phase and failed rows offer return-to-home", () => {
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

test("mobile pending delivery arrows survive a dropped iOS compatibility click", () => {
  const start = composer.indexOf("function PendingRow(");
  const end = composer.indexOf("function StopConfirmDialog", start);
  assert(start >= 0 && end > start);
  const body = composer.slice(start, end);
  assertEquals(body.match(/reliableTouch=\{!desktop\}/g)?.length, 2);
  assert(
    body.includes("networkAction={() => activateDraft(sessionId, message.id)}"),
  );
  assert(
    body.includes(
      "networkAction={() => requestSendQueued(sessionId, message.id)}",
    ),
  );
});

test("tool UI selection uses fill instead of purple leading rails", () => {
  assertEquals(transcript.includes("borderLeft: 2"), false);
  assertEquals(transcript.includes("borderLeft: 3"), false);
  assertEquals(
    transcript.includes(
      "`inset 3px 0 0 ${alpha(theme.palette.primary.main, 0.78)}`",
    ),
    false,
  );
});

test("MessagePreview renders cowboy-att tokens as composer inline images", () => {
  assert(previewSource.includes("inlineImageField"));
  assert(previewSource.includes("seedInlineAttachments(attachments)"));
  assertEquals(
    previewSource.includes("compactForPreview(stripImageTokens(text))"),
    false,
  );
});

test("mdlive leaves cowboy-att images for the inline widget instead of hiding them", () => {
  assert(inlinePreviewSource.includes("!imageText.includes('cowboy-att:')"));
});

test("confirmed user rows stay hidden while the optimistic image bubble is up", () => {
  assert(transcript.includes("optimisticCmids.has(item.cmid)"));
  assertEquals(transcript.includes("overlayHidesTailHumanKey"), false);
  assert(transcript.includes("applySendImagePreviews(chunks, cmid)"));
  assert(transcript.includes("retainUnpresentedOptimistic("));
  assert(transcript.includes("presentedTimeline !== timeline,"));
  assert(transcript.includes("hasNewerLiveUserItem("));
});

test("an echoed transcript send retires once its turn starts working", async () => {
  const store = await readFile(new URL("./store.ts", import.meta.url), "utf8");
  const start = store.indexOf('case "event": {');
  const end = store.indexOf('case "config_options": {', start);
  assert(start >= 0 && end > start);
  const body = store.slice(start, end);
  const remember = body.indexOf("echoedOptimisticCmids.add(cmid)");
  const retire = body.indexOf("envelopeCompletesPromptEcho(env)");
  assert(body.indexOf("reconcileReadyOptimistic(") < remember);
  assert(remember >= 0 && retire > remember);
  assert(body.indexOf("setState({", retire) > retire);
});

test("failed transcript sends offer return to the list they left", () => {
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

test("local content paints and reveals before the durable transport barrier resolves", async () => {
  const store = await readFile(new URL("./store.ts", import.meta.url), "utf8");
  const addStart = store.indexOf("async function qAdd(");
  const addEnd = store.indexOf("export function retryQueued", addStart);
  const add = store.slice(addStart, addEnd);
  assert(add.indexOf('qStatus.set(cmid, "committing")') >= 0);
  assert(
    add.indexOf("revealPendingArrival({") <
      add.indexOf("await mutateQueueDurably"),
  );
  assert(add.indexOf("await mutateQueueDurably") >= 0);
  assert(add.includes("rememberSendImagePreviews(cmid, attachments, text)"));

  const activateStart = store.indexOf("export async function activateDraft(");
  const activateEnd = store.indexOf(
    "export function activateAllDrafts",
    activateStart,
  );
  const activate = store.slice(activateStart, activateEnd);
  assert(
    activate.indexOf('qStatus.set(opId, "committing")') <
      activate.indexOf("await mutateQueueDurably"),
  );
  assert(activate.includes("row: presented"));
  assert(activate.includes("destination: dest"));
  assertEquals(activate.includes("await optimisticMessage("), false);
  assertEquals(activate.includes("await discardQueued("), false);
  assert(store.includes("reconcileReadyOptimistic("));
  assert(store.includes("waitForPresentedState("));

  const commitStart = store.indexOf("function commitQueue(");
  const commitEnd = store.indexOf("function armQTimers", commitStart);
  const commit = store.slice(commitStart, commitEnd);
  assert(commit.includes("setInteractiveState({"));
  assert(commit.includes("pendingRowStatuses"));

  const editStart = store.indexOf("async function editPendingRow(");
  const editEnd = store.indexOf(
    "export async function requestSendQueued",
    editStart,
  );
  const edit = store.slice(editStart, editEnd);
  assert(
    edit.indexOf('qStatus.set(opId, "committing")') <
      edit.indexOf("await mutateQueueDurably"),
  );
  assert(edit.includes("qStatus.delete(opId)"));

  const sendQueuedStart = store.indexOf(
    "export async function requestSendQueued(",
  );
  const sendQueuedEnd = store.indexOf(
    "export async function forcePushQueued",
    sendQueuedStart,
  );
  const sendQueued = store.slice(sendQueuedStart, sendQueuedEnd);
  assert(sendQueued.includes("qStatus.delete(opId)"));
  assert(sendQueued.includes("qStatus.delete(echoCmid)"));
});

test("unfocused pending draft activation actively recovers a missing server id", async () => {
  const store = await readFile(new URL("./store.ts", import.meta.url), "utf8");
  const transport = store.slice(
    store.indexOf("function transmitQueueMutation("),
    store.indexOf("function qClient("),
  );
  assert(transport.includes("if (command === null && isConnected())"));
  assert(transport.includes("void hydrateSession(sessionId)"));
  const hydration = store.slice(
    store.indexOf("async function hydrateSession("),
    store.indexOf("export function retrySessionHydration("),
  );
  assert(
    hydration.includes("const retryDraft = needsDraftSource(sessionId) &&"),
  );
  assert(hydration.includes("retryTranscript || retryDraft"));
  assert(hydration.includes("retryableFailure = needsDraftSource(sessionId)"));
  assert(hydration.includes('qStatus.set(mutation.id, "failed")'));
  const discard = store.slice(
    store.indexOf("async function discardQueueMutationDurably("),
    store.indexOf("function pendingNamed("),
  );
  assert(discard.includes("discardDurableDelivery(store, cmid"));
  assert(discard.includes('qStatus.set(cmid, "failed")'));
});

test("pending preview Show more toggles on a stationary touch", () => {
  assert(
    previewSource.includes(
      "const disclosureTap = useReliableTouchTap<HTMLButtonElement>(() =>",
    ),
  );
  const button = previewSource.slice(
    previewSource.indexOf("{...disclosureTap}"),
    previewSource.indexOf('{expanded ? "Show less" : "Show more"}'),
  );
  assert(button.includes("e.stopPropagation();"));
  assert(button.includes("disclosureTap.onClick(e);"));
  assertEquals(button.includes("setExpanded("), false);
});
