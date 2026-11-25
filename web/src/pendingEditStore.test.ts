import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import type { Attachment } from "./attachments.ts";
import {
  claimOrphanedPendingEdits,
  clearPendingEdit,
  discardPendingEdits,
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

test("pending row edit survives local flush and restores by target id", () => {
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

test("already committed recovery retires instead of reopening the editor", () => {
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

test("unencoded paste placeholders never become broken recovered images", () => {
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

test("an edit whose server row drained is claimed once for parked-draft recovery", () => {
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

test("discardPendingEdits removes only the target session from memory and localStorage", () => {
  const gone = "pending-edit-discard-gone";
  const kept = "pending-edit-discard-kept";
  const entries = new Map<string, string>();
  const fakeStorage: Storage = {
    get length() {
      return entries.size;
    },
    key: (index: number) => [...entries.keys()][index] ?? null,
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => {
      entries.set(key, value);
    },
    removeItem: (key: string) => {
      entries.delete(key);
    },
    clear: () => {
      entries.clear();
    },
  };
  const globals = globalThis as { localStorage?: Storage };
  const previousStorage = globals.localStorage;
  globals.localStorage = fakeStorage;
  try {
    setPendingEdit({
      sessionId: gone,
      kind: "draft",
      id: "d1",
      text: "delete me",
      attachments: [],
      baseText: "before",
      baseAttachments: [],
    });
    setPendingEdit({
      sessionId: kept,
      kind: "queued",
      id: "q1",
      text: "keep me",
      attachments: [],
      baseText: "before",
      baseAttachments: [],
    });
    flushPendingEdits();
    // Also seed a mirror written by another tab that is not in this tab's memory map.
    const otherTabKey = `cowboy:pending-edit:${JSON.stringify([gone, "queued", "from-other-tab"])}`;
    const keptKey = `cowboy:pending-edit:${JSON.stringify([kept, "queued", "q1"])}`;
    fakeStorage.setItem(
      otherTabKey,
      JSON.stringify({
        sessionId: gone,
        kind: "queued",
        id: "from-other-tab",
        text: "other tab",
        attachments: [],
        baseFingerprint: "[]",
        recoveryCmid: "pending-edit-other",
        updatedAt: Date.now(),
      }),
    );

    discardPendingEdits(gone);

    assertEquals(getPendingEdit(gone, "draft", "d1"), null);
    assertEquals(fakeStorage.getItem(otherTabKey), null);
    assertEquals(getPendingEdit(kept, "queued", "q1")?.text, "keep me");
    assertEquals(fakeStorage.getItem(keptKey) !== null, true);

    // A stale peer tab with a buffered dirty edit must not rewrite localStorage
    // after another tab records permanent removal in the shared fence.
    const gonePeer = `sess-gone-peer-${Date.now()}`;
    setPendingEdit({
      sessionId: gonePeer,
      kind: "draft",
      id: "d-peer",
      text: "stale peer edit",
      attachments: [],
      baseText: "before",
      baseAttachments: [],
    });
    fakeStorage.setItem("cowboy:removed-sessions", JSON.stringify([gone, gonePeer]));
    flushPendingEdits();
    const gonePeerKey = `cowboy:pending-edit:${JSON.stringify([gonePeer, "draft", "d-peer"])}`;
    assertEquals(fakeStorage.getItem(gonePeerKey), null);

    clearPendingEdit(kept, "queued", "q1");
  } finally {
    globals.localStorage = previousStorage;
  }
});
