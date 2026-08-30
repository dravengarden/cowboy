import { assertEquals } from "jsr:@std/assert";
import type { Attachment } from "./attachments.ts";
import {
  claimOrphanedPendingEdits,
  clearPendingEdit,
  finishOrphanedPendingEdit,
  flushPendingEdits,
  getPendingEdit,
  recoverPendingEditId,
  setPendingEdit,
} from "./pendingEditStore.ts";

function imageAttachment(id: string, pending = false): Attachment {
  return {
    id,
    name: `${id}.png`,
    mimeType: "image/png",
    isImage: true,
    block: { type: "image", data: pending ? "" : "cGl4ZWw=", mimeType: "image/png" },
    ...(pending ? { pending: true } : {}),
  };
}

Deno.test("pending row edit survives local flush and restores by target id", () => {
  const sessionId = "pending-edit-reload";
  const id = "draft-1";
  const attachment = imageAttachment("image-1");
  setPendingEdit({
    sessionId,
    kind: "draft",
    id,
    text: "changed ![image](cowboy-att:image-1)",
    attachments: [attachment],
    baseText: "before",
    baseAttachments: [],
  });
  flushPendingEdits();

  assertEquals(getPendingEdit(sessionId, "draft", id)?.text, "changed ![image](cowboy-att:image-1)");
  assertEquals(
    recoverPendingEditId(sessionId, "draft", [{ id, text: "before", attachments: [] }]),
    id,
  );
  clearPendingEdit(sessionId, "draft", id);
});

Deno.test("already committed recovery retires instead of reopening the editor", () => {
  const sessionId = "pending-edit-committed";
  const id = "queued-1";
  setPendingEdit({
    sessionId,
    kind: "queued",
    id,
    text: "saved",
    attachments: [],
    baseText: "before",
    baseAttachments: [],
  });

  assertEquals(
    recoverPendingEditId(sessionId, "queued", [{ id, text: "saved", attachments: [] }]),
    null,
  );
  assertEquals(getPendingEdit(sessionId, "queued", id), null);
});

Deno.test("unencoded paste placeholders never become broken recovered images", () => {
  const sessionId = "pending-edit-paste";
  const id = "draft-paste";
  setPendingEdit({
    sessionId,
    kind: "draft",
    id,
    text: "kept text ![pending](cowboy-att:pending-image)",
    attachments: [imageAttachment("pending-image", true)],
    baseText: "before",
    baseAttachments: [],
  });

  assertEquals(
    getPendingEdit(sessionId, "draft", id)?.text.trimEnd(),
    "kept text",
  );
  assertEquals(getPendingEdit(sessionId, "draft", id)?.attachments, []);
  clearPendingEdit(sessionId, "draft", id);
});

Deno.test("an edit whose server row drained is claimed once for parked-draft recovery", () => {
  const sessionId = "pending-edit-orphan";
  const id = "queued-gone";
  setPendingEdit({
    sessionId,
    kind: "queued",
    id,
    text: "unsaved replacement",
    attachments: [],
    baseText: "already dispatched original",
    baseAttachments: [],
  });

  const first = claimOrphanedPendingEdits(
    sessionId,
    new Set(),
    new Set(),
  );
  assertEquals(first.length, 1);
  assertEquals(first[0]?.recoveryCmid.startsWith("pending-edit-"), true);
  assertEquals(
    claimOrphanedPendingEdits(sessionId, new Set(), new Set()),
    [],
  );

  finishOrphanedPendingEdit(first[0]!, false);
  const retry = claimOrphanedPendingEdits(sessionId, new Set(), new Set());
  assertEquals(retry[0]?.recoveryCmid, first[0]?.recoveryCmid);
  finishOrphanedPendingEdit(retry[0]!, true);
  assertEquals(getPendingEdit(sessionId, "queued", id), null);
});
