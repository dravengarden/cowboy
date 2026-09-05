import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import type { Envelope } from "../protocol";
import type { ExploreSessionState } from "./contextClear";
import {
  newlySubmittedQuestionPage,
  optimisticQuestionKey,
  projectQuestionPages,
  reconcileOptimisticPageState,
} from "./optimisticPages";
import {
  groupQuestionPages,
  indexedQuestionPagePosition,
  mergeQuestionPageDirectory,
} from "./questionPages";

const pending = { id: "opt-local-1", cmid: "local-1", text: "Next question" };
const localId = optimisticQuestionKey(pending);

function prompt(seq: number, text: string, cmid?: string): Envelope {
  return {
    session_id: "s1",
    seq,
    kind: "update",
    update: {
      sessionUpdate: "user_message_chunk",
      content: { type: "text", text },
    },
    ...(cmid === undefined ? {} : { cmid }),
  };
}

const history: Envelope[] = [prompt(10, "Previous question"), {
  session_id: "s1",
  seq: 11,
  kind: "update",
  update: {
    sessionUpdate: "agent_message_chunk",
    content: { type: "text", text: "Previous answer" },
  },
}, { session_id: "s1", seq: 12, kind: "turn_end", stop_reason: "end_turn" }];

function state(patch: Partial<ExploreSessionState> = {}): ExploreSessionState {
  return {
    projection: "explore",
    pageId: "10",
    pageStartId: null,
    pageLoadingId: null,
    transitionAnchorKey: null,
    followTailRequested: false,
    pageParents: {},
    pendingFollowUp: null,
    ...patch,
  };
}

Deno.test("a local send starts a separate question page without any server event", () => {
  const before = projectQuestionPages(history, []);
  const after = projectQuestionPages(history, [pending]);
  assertEquals(before.pages.length, 1);
  assertEquals(after.pages.length, 2);
  assertEquals(after.pages[0], before.pages[0]);
  assertEquals(after.pages[1]?.itemKeys, [localId]);
  assertEquals(after.pages[1]?.title, "Next question");
  assertEquals(after.pages[1]?.questionCount, 1);
  assertEquals(newlySubmittedQuestionPage(["10"], after.pages, false), localId);
  assertEquals(history.length, 3);
});

Deno.test("a 51-page stale index immediately shows 52/52 and survives confirmation", () => {
  const index = [{ id: "10", title: "Previous question", ordinal: 51 }];
  const local = projectQuestionPages(history, [pending]);
  const directory = mergeQuestionPageDirectory(index, local.pages, 51);
  assertEquals(directory.at(-1)?.ordinal, 52);
  assertEquals(indexedQuestionPagePosition(directory, localId), {
    ordinal: 52,
    previousId: "10",
    nextId: undefined,
  });
  const confirmed = projectQuestionPages([
    ...history,
    prompt(20, pending.text, pending.cmid),
  ], []);
  const confirmedDirectory = mergeQuestionPageDirectory(
    index,
    confirmed.pages,
    51,
  );
  assertEquals(confirmedDirectory.at(-1)?.ordinal, 52);
  assertEquals(confirmedDirectory.at(-1)?.id, "20");
});

Deno.test("an echo and its pending outbox twin produce exactly one page", () => {
  const projected = projectQuestionPages([
    ...history,
    prompt(20, pending.text, pending.cmid),
  ], [pending]);
  assertEquals(projected.pages.map((page) => page.id), ["10", "20"]);
  assertEquals(projected.aliases.get(localId), "20");
  const selected = reconcileOptimisticPageState(
    state({ pageId: localId }),
    projected.aliases,
    new Set(projected.pages.map((page) => page.id)),
  );
  assertEquals(selected.pageId, "20");
  assertEquals(selected.pageLoadingId, null);
});

Deno.test("identical question text is never used to reconcile different sends", () => {
  const other = { ...pending, id: "opt-local-2", cmid: "local-2" };
  const projected = projectQuestionPages([
    ...history,
    prompt(20, pending.text, pending.cmid),
  ], [pending, other]);
  assertEquals(projected.pages.map((page) => page.id), [
    "10",
    "20",
    optimisticQuestionKey(other),
  ]);
});

Deno.test("delivery phases retain one local page and do not repeat the navigation intent", () => {
  for (const status of ["committing", "pending", "failed"]) {
    const message = { ...pending, status };
    const projected = projectQuestionPages(history, [message]);
    assertEquals(projected.pages.at(-1)?.id, localId);
    assertEquals(
      newlySubmittedQuestionPage(["10", localId], projected.pages, false),
      null,
    );
  }
});

Deno.test("failure recovery or queue rerouting removes only the vanished local selection", () => {
  const next = reconcileOptimisticPageState(
    state({
      pageId: localId,
      pageStartId: localId,
      pageLoadingId: localId,
      transitionAnchorKey: localId,
    }),
    new Map(),
    new Set(["10"]),
  );
  assertEquals(next, state({ pageId: null }));
  assertEquals(projectQuestionPages(history, []).pages.length, 1);
});

Deno.test("confirmation does not steal a reader's older-page selection", () => {
  const previous = state({ pageId: "10" });
  assertStrictEquals(
    reconcileOptimisticPageState(
      previous,
      new Map([[localId, "20"]]),
      new Set(["10", "20"]),
    ),
    previous,
  );
});

Deno.test("an unconfirmed local page stays selected across a retry", () => {
  const previous = state({ pageId: localId });
  assertStrictEquals(
    reconcileOptimisticPageState(previous, new Map(), new Set(["10", localId])),
    previous,
  );
});

Deno.test("continuation grouping migrates from the local cmid to the durable page", () => {
  const previous = state({ pageParents: { [localId]: "10" } });
  const projected = projectQuestionPages([
    ...history,
    prompt(20, pending.text, pending.cmid),
  ], []);
  const next = reconcileOptimisticPageState(
    previous,
    projected.aliases,
    new Set(["10", "20"]),
  );
  assertEquals(next.pageParents, { "20": "10" });
  assertEquals(groupQuestionPages(projected.pages, next.pageParents).length, 1);
  assertEquals(
    newlySubmittedQuestionPage(
      ["10"],
      projectQuestionPages(history, [pending]).pages,
      true,
    ),
    null,
  );
});

Deno.test("a pending continuation retains its target and known IDs when an echo lands", () => {
  const previous = state({
    pendingFollowUp: { targetPageId: localId, knownPageIds: ["10", localId] },
  });
  const next = reconcileOptimisticPageState(
    previous,
    new Map([[localId, "20"]]),
    new Set(["10", "20"]),
  );
  assertEquals(next.pendingFollowUp, {
    targetPageId: "20",
    knownPageIds: ["10", "20"],
  });
});

Deno.test("out-of-order confirmation cannot replace a newer local selection", () => {
  const later = { ...pending, cmid: "local-2" };
  const laterId = optimisticQuestionKey(later);
  const projected = projectQuestionPages([
    ...history,
    prompt(20, pending.text, pending.cmid),
  ], [later]);
  const next = reconcileOptimisticPageState(
    state({ pageId: laterId }),
    projected.aliases,
    new Set(projected.pages.map((page) => page.id)),
  );
  assertEquals(next.pageId, laterId);
});

Deno.test("attachment-only sends create a page and compaction commands do not", () => {
  const attachment = projectQuestionPages(history, [{ ...pending, text: "" }]);
  assertEquals(attachment.pages.length, 2);
  assertEquals(attachment.pages.at(-1)?.title, "Page 2");
  for (const command of ["/compact", "/compress"]) {
    const projected = projectQuestionPages(history, [{
      ...pending,
      text: command,
    }]);
    assertEquals(projected.pages.length, 1);
    assertEquals(projected.pages[0]?.itemKeys.includes(localId), true);
    assertEquals(
      newlySubmittedQuestionPage(["10"], projected.pages, false),
      null,
    );
  }
});

Deno.test("page roots and delivery bubbles use one shared optimistic key", async () => {
  const transcript = await Deno.readTextFile(
    new URL("../Transcript.tsx", import.meta.url),
  );
  const store = await Deno.readTextFile(
    new URL("../store.ts", import.meta.url),
  );
  assertEquals(
    transcript.includes("visibleItemKeys.has(optimisticQuestionKey(message))"),
    true,
  );
  assertEquals(
    transcript.includes("data-key={optimisticQuestionKey(om)}"),
    true,
  );
  assertEquals(
    transcript.includes('status !== "exited" || optimisticMsgs.length === 0'),
    true,
  );
  assertEquals(
    store.includes("if (isQuestionPageLoaded(sessionId, pageId)) return true;"),
    true,
  );
});

Deno.test("discarding a durable chat send also removes its local page overlay", async () => {
  const store = await Deno.readTextFile(new URL("../store.ts", import.meta.url));
  const start = store.indexOf("export async function discardMessage(");
  const end = store.indexOf("function optimisticMessage(", start);
  const discard = store.slice(start, end);
  assertEquals(discard.includes("await discardQueued(sessionId, pending.id)"), true);
  assertEquals(discard.includes('patchMessage(sessionId, cmid, "drop")'), true);
  assertEquals(discard.includes("return;"), false);
});
