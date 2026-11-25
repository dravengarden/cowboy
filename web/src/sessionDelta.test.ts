import { expect, test } from "bun:test";
import type { Envelope, SessionMeta } from "./protocol";
import {
  boundSocketUrl,
  liveEventsAfterSnapshot,
  removeSessionById,
  shouldDiscardStaleReplica,
  upsertSession,
} from "./sessionDelta";

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

test("liveEventsAfterSnapshot preserves newer live events without hiding the gap behind a replica", () => {
  const cachedReplica = [message(1, "old-1"), message(2, "old-2")];
  const existingWithLive = [...cachedReplica, message(100, "live-100"), message(102, "live-102")];
  const freshSnapshot = [message(100, "snap-100"), message(101, "snap-101")];

  // Comparing the original IndexedDB tail still detects the 2..100 gap even after
  // live event 100 arrived before the HTTP snapshot.
  expect(shouldDiscardStaleReplica("replica", cachedReplica, freshSnapshot)).toBe(true);
  // Newer live events at or after the snapshot start survive the replica discard.
  expect(
    liveEventsAfterSnapshot(existingWithLive, cachedReplica, freshSnapshot).map((e) => e.seq),
  ).toEqual([100, 102]);
  // Conflicting epochs drop everything from the old epoch.
  expect(
    liveEventsAfterSnapshot([message(20, "old"), message(21, "old")], [message(20, "old")], [message(5, "new")]),
  ).toEqual([]);
});
