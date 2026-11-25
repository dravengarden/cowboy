import { expect, test } from "bun:test";
import type { Envelope, SessionMeta } from "./protocol";
import {
  boundSocketUrl,
  liveEventsToReplayAfterSnapshot,
  removeSessionById,
  shouldDiscardStaleReplica,
  upsertSession,
} from "./sessionDelta";
import { applyEnvelope, mergeEvents } from "./storeTimeline";

function session(id: string, status: SessionMeta["status"] = "running"): SessionMeta {
  return {
    id,
    provider: "codex",
    machine_id: "hawk",
    cwd: "/tmp",
    title: id,
    status,
    origin: "web",
    paused: false,
    context_used: 0,
    context_size: 0,
  };
}

function message(seq: number, text: string): Envelope {
  return {
    session_id: "s1",
    seq,
    kind: "update",
    update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text } },
  };
}

test("boundSocketUrl opts into lazy bootstrap and incremental session deltas", () => {
  const url = new URL(
    boundSocketUrl(
      { protocol: "https:", host: "cowboy.test" },
      "ds-1",
      "client-1234567890123456",
      "browser",
    ),
  );
  expect(url.protocol).toBe("wss:");
  expect(url.pathname).toBe("/ws");
  expect(url.searchParams.get("bootstrap")).toBe("lazy");
  expect(url.searchParams.get("session_deltas")).toBe("1");
  expect(url.searchParams.get("dataset")).toBe("ds-1");
});

test("upsertSession replaces in place and appends new sessions at the tail", () => {
  const initial = [session("a"), session("b")];
  const updated = upsertSession(initial, session("a", "busy"));
  expect(updated.previous?.status).toBe("running");
  expect(updated.sessions.map((s) => `${s.id}:${s.status}`)).toEqual([
    "a:busy",
    "b:running",
  ]);
  const created = upsertSession(updated.sessions, session("c"));
  expect(created.previous).toBeUndefined();
  expect(created.sessions.map((s) => s.id)).toEqual(["a", "b", "c"]);
});

test("removeSessionById removes matching ids without mutating the input", () => {
  const initial = [session("a"), session("b")];
  const removed = removeSessionById(initial, "a");
  expect(removed.removed).toBe(true);
  expect(removed.sessions.map((s) => s.id)).toEqual(["b"]);
  expect(initial.map((s) => s.id)).toEqual(["a", "b"]);
  expect(removeSessionById(initial, "missing").removed).toBe(false);
});

test("shouldDiscardStaleReplica discards non-overlapping replicas without disabling live gap-fill", () => {
  const stale = [message(10, "old"), message(11, "old")];
  const freshTail = [message(500, "new"), message(501, "new")];
  const overlapping = [message(11, "old"), message(12, "next")];

  // Stale IndexedDB paint cache with a non-overlapping gap is discarded so the
  // client does not page 32 history chunks just to bridge yesterday's cache.
  expect(shouldDiscardStaleReplica("replica", stale, freshTail)).toBe(true);
  // Overlapping replica merges smoothly without discarding.
  expect(shouldDiscardStaleReplica("replica", stale, overlapping)).toBe(false);
  // Live sessions still preserve their in-memory prefix and run fillSnapshotJoinGap.
  expect(shouldDiscardStaleReplica("live", stale, freshTail)).toBe(false);
  // Conflicting epoch on replica is still discarded.
  expect(shouldDiscardStaleReplica("replica", [message(20, "a")], [message(10, "b")])).toBe(true);
  // Empty cached tail carries a null pagination cursor and must reseed from the live snapshot.
  expect(shouldDiscardStaleReplica("replica", [], freshTail)).toBe(true);
});

test("liveEventsToReplayAfterSnapshot replays newer absorbed chunks without overwriting or duplicating", () => {
  const buffered = [message(100, "hello"), message(101, " world")];
  // Snapshot captured right after seq 100 (`next_seq = 101`): seq 101 must
  // replay onto row 100 so the absorbed text is preserved.
  const snapBefore101 = [message(100, "hello")];
  const toReplay = liveEventsToReplayAfterSnapshot(buffered, snapBefore101, 101, null);
  expect(toReplay.map((e) => e.seq)).toEqual([101]);
  let timelines = mergeEvents(new Map(), "s1", snapBefore101);
  for (const env of toReplay) timelines = applyEnvelope(timelines, env);
  const row = timelines.get("s1")?.[0];
  expect(row?.kind === "update" ? row.update.content?.text : "").toBe("hello world");

  // Snapshot captured after seq 101 (`next_seq = 102`): seq 101 is already
  // folded into row 100 and must NOT be replayed again.
  const snapAfter101 = [message(100, "hello world")];
  expect(liveEventsToReplayAfterSnapshot(buffered, snapAfter101, 102, null)).toEqual([]);

  // Older Controller without next_seq falls back to strictly past incoming max.
  expect(liveEventsToReplayAfterSnapshot(buffered, snapBefore101, undefined, null).map((e) => e.seq))
    .toEqual([101]);

  // Cleared boundary drops earlier buffered events.
  expect(liveEventsToReplayAfterSnapshot([message(99, "old"), message(105, "new")], [], 90, 102).map((e) => e.seq))
    .toEqual([105]);
});
