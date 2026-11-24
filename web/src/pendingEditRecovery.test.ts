import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { bindPendingEditRecovery, recoverPendingEditsFrom } from "./pendingEditRecovery.ts";
import { getPendingEdit, type PendingEditRecord, setPendingEdit } from "./pendingEditStore.ts";

function edit(sessionId: string, text: string): void {
  setPendingEdit({
    sessionId,
    kind: "queued",
    id: "row-1",
    text,
    attachments: [],
    baseText: "before",
    baseAttachments: [],
  });
}

test("an older copy parked under the shared cmid does not retire a newer edit", async () => {
  const parked: PendingEditRecord[] = [];
  bindPendingEditRecovery({
    pendingMutationIds: () => [],
    park: (_session, record) => {
      parked.push(record);
      return Promise.resolve();
    },
  });
  edit("cmid-older", "newer text");
  const cmid = getPendingEdit("cmid-older", "queued", "row-1")!.recoveryCmid;
  // Another tab parked the older text it had hydrated, under the same cmid.
  recoverPendingEditsFrom("cmid-older", {
    queue: [],
    drafts: [{ id: "d1", cmid, text: "older text" }],
  });
  await Promise.resolve();
  assertEquals(parked.map((record) => record.text), ["newer text"]);
  assertEquals(parked[0]!.recoveryCmid === cmid, false);

  // The same text under that cmid is this edit's own park: it retires.
  edit("cmid-same", "same text");
  const own = getPendingEdit("cmid-same", "queued", "row-1")!.recoveryCmid;
  recoverPendingEditsFrom("cmid-same", {
    queue: [],
    drafts: [{ id: "d2", cmid: own, text: "same text" }],
  });
  assertEquals(getPendingEdit("cmid-same", "queued", "row-1"), null);
  assertEquals(parked.length, 1);
});
