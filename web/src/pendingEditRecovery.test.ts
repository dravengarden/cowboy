import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  bindPendingEditRecovery,
  pendingEditParkCmid,
  recoverPendingEditsFrom,
} from "./pendingEditRecovery.ts";
import { getPendingEdit, type PendingEditRecord, setPendingEdit } from "./pendingEditStore.ts";

function edit(sessionId: string, text: string): PendingEditRecord {
  setPendingEdit({
    sessionId,
    kind: "queued",
    id: "row-1",
    text,
    attachments: [],
    baseText: "before",
    baseAttachments: [],
  });
  return getPendingEdit(sessionId, "queued", "row-1")!;
}

test("only a draft of exactly this content retires a recovered edit", async () => {
  const parked: [string, string][] = [];
  bindPendingEditRecovery({
    pendingMutationIds: () => [],
    park: (_session, record, cmid) => {
      parked.push([record.text, cmid]);
      return Promise.resolve();
    },
  });
  // Another tab parked the older copy it had hydrated.
  const older = edit("receipt", "older text");
  const olderCmid = pendingEditParkCmid(older);
  const newer = edit("receipt", "newer text");
  assertEquals(newer.recoveryCmid, older.recoveryCmid);
  recoverPendingEditsFrom("receipt", { queue: [], drafts: [{ id: "d1", cmid: olderCmid, text: "older text" }] });
  await Promise.resolve();
  assertEquals(parked, [["newer text", pendingEditParkCmid(newer)]]);
  assertEquals(pendingEditParkCmid(newer) === olderCmid, false);

  // That content's own draft is its receipt; tabs holding it converge on it.
  const own = edit("own", "same text");
  recoverPendingEditsFrom("own", {
    queue: [],
    drafts: [{ id: "d2", cmid: pendingEditParkCmid(own), text: "same text" }],
  });
  assertEquals(getPendingEdit("own", "queued", "row-1"), null);
  assertEquals(parked.length, 1);
});

test("rewriting an unchanged buffer keeps its stamp", async () => {
  const first = edit("stamp", "same");
  await new Promise((resolve) => setTimeout(resolve, 5));
  assertEquals(edit("stamp", "same").updatedAt, first.updatedAt);
  assertEquals(edit("stamp", "changed").updatedAt > first.updatedAt, true);
});

test("in-flight orphan park preserves and recovers a newer revision", async () => {
  const parked: string[] = [];
  let releaseFirst!: () => void;
  let calls = 0;
  bindPendingEditRecovery({
    pendingMutationIds: () => [],
    park: (_session, record) => {
      parked.push(record.text);
      calls += 1;
      if (calls === 1) {
        return new Promise<void>((resolve) => {
          releaseFirst = resolve;
        });
      }
      return Promise.resolve();
    },
  });

  edit("in-flight-newer", "rev-1");
  recoverPendingEditsFrom("in-flight-newer", { queue: [], drafts: [] });
  assertEquals(parked, ["rev-1"]);

  // User types a newer revision while rev-1's orphan park is still in flight.
  edit("in-flight-newer", "rev-2");
  releaseFirst();
  for (let i = 0; i < 6; i += 1) await Promise.resolve();

  assertEquals(parked, ["rev-1", "rev-2"]);
  assertEquals(getPendingEdit("in-flight-newer", "queued", "row-1"), null);
});
