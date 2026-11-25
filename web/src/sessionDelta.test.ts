import { expect, test } from "bun:test";
import type { Envelope, SessionMeta } from "./protocol";
import {
  authoritativeLinkMetaMap,
  boundSocketUrl,
  classifyReplicaSnapshot,
  diffAuthoritativeLinkMeta,
  dropStaleReplicaPrefix,
  removeSessionById,
  upsertSession,
} from "./sessionDelta";
import {
  isSessionRemoved,
  markSessionRemoved,
  setRemovalFenceDataset,
} from "./removedSessionFence";
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

test("classifyReplicaSnapshot and dropStaleReplicaPrefix evict stale caches without dropping live events", () => {
  const stale = [message(10, "old"), message(11, "old")];
  const freshTail = [message(500, "new"), message(501, "new")];
  const overlapping = [message(11, "old"), message(12, "next")];

  expect(classifyReplicaSnapshot("replica", stale, freshTail)).toBe("stale_prefix");
  expect(classifyReplicaSnapshot("replica", stale, overlapping)).toBe("keep");
  expect(classifyReplicaSnapshot("live", stale, freshTail)).toBe("keep");
  expect(classifyReplicaSnapshot("replica", [message(20, "a")], [message(10, "b")])).toBe("conflict");
  expect(classifyReplicaSnapshot("replica", [], freshTail)).toBe("reseed_empty");

  const withLive = [...stale, message(500, "live"), message(502, "newer")];
  expect(dropStaleReplicaPrefix(withLive, stale).map((e) => e.seq)).toEqual([500, 502]);

  // A cached tool row that absorbed a live tool_call_update after replicaMax
  // survives prefix trimming even though its anchor seq <= replicaMax.
  const cachedTool: Envelope = {
    session_id: "s1",
    seq: 10,
    kind: "update",
    update: { sessionUpdate: "tool_call", toolCallId: "t-1", status: "in_progress" },
  };
  const liveToolDone: Envelope = {
    session_id: "s1",
    seq: 505,
    kind: "update",
    update: { sessionUpdate: "tool_call_update", toolCallId: "t-1", status: "completed" },
  };
  const cachedReplica = [cachedTool, message(11, "old")];
  const afterLiveUpdate = applyEnvelope(new Map([["s1", cachedReplica]]), liveToolDone).get("s1")!;
  const kept = dropStaleReplicaPrefix(afterLiveUpdate, cachedReplica);
  expect(kept.map((e) => e.seq)).toEqual([10]);
  expect(kept[0]?.kind === "update" ? kept[0].update.status : "").toBe("completed");
});

test("authoritativeLinkMetaMap and diffAuthoritativeLinkMeta track trashed sessions, Web titles, and session_updated titles", () => {
  const a = session("a");
  const trashedB: SessionMeta = { ...session("b"), trashed_at_ms: 1_700_000_000_000 };
  const prev = authoritativeLinkMetaMap([a, trashedB], { a: "Seeded Title" });
  // Auto/legacy rename via session_updated changes SessionMeta.title even when
  // titleSync.base() still holds "Seeded Title".
  const autoRenamedA: SessionMeta = { ...a, title: "Auto Renamed" };
  const afterAuto = authoritativeLinkMetaMap([autoRenamedA, trashedB], { a: "Seeded Title" });
  expect(diffAuthoritativeLinkMeta(prev, afterAuto)).toEqual(["a"]);
  // Authoritative Web title update + closing a trashed session.
  const next = authoritativeLinkMetaMap(
    [autoRenamedA, { ...trashedB, closing: true }],
    { a: "Committed Title" },
  );
  expect(diffAuthoritativeLinkMeta(afterAuto, next).sort()).toEqual(["a", "b"]);
  expect(diffAuthoritativeLinkMeta(next, next)).toEqual([]);
  // Removing a trashed session is detected too.
  const afterPurge = authoritativeLinkMetaMap([autoRenamedA], { a: "Committed Title" });
  expect(diffAuthoritativeLinkMeta(next, afterPurge)).toEqual(["b"]);
});

test("mergeEvents preserves a live tool completion over an overlapping older snapshot", () => {
  const started: Envelope = {
    session_id: "s1",
    seq: 10,
    kind: "update",
    update: { sessionUpdate: "tool_call", toolCallId: "t-live", status: "in_progress" },
  };
  const liveCompletion: Envelope = {
    session_id: "s1",
    seq: 50,
    kind: "update",
    update: { sessionUpdate: "tool_call_update", toolCallId: "t-live", status: "completed" },
  };
  const liveTimeline = applyEnvelope(new Map([["s1", [started]]]), liveCompletion);
  // Overlapping snapshot captured at seq 45 still has tool_call as in_progress.
  const olderSnapshot = [started, message(45, "mid-turn")];
  const mergedOlder = mergeEvents(liveTimeline, "s1", olderSnapshot).get("s1")!;
  const toolAfterOlder = mergedOlder.find((e) => e.seq === 10);
  expect(toolAfterOlder?.kind === "update" ? toolAfterOlder.update.status : "").toBe("completed");
  // Newer canonical snapshot captured at seq 60 (past seq 50) replaces with canonical row.
  const canonicalDone: Envelope = {
    session_id: "s1",
    seq: 10,
    kind: "update",
    update: { sessionUpdate: "tool_call", toolCallId: "t-live", status: "completed", title: "canonical" },
  };
  const mergedNewer = mergeEvents(liveTimeline, "s1", [canonicalDone, message(60, "done")]).get("s1")!;
  const toolAfterNewer = mergedNewer.find((e) => e.seq === 10);
  expect(toolAfterNewer?.kind === "update" ? toolAfterNewer.update.title : "").toBe("canonical");

  // Streamed message chunks: a canonical snapshot carrying missed chunk 12
  // ("Hello world!") must beat a shorter live row ("Hello world") even when
  // the canonical row stays anchored at seq 10.
  const chunk10 = message(10, "Hello");
  const chunk11 = message(11, " world");
  const liveChunks = applyEnvelope(new Map([["s1", [chunk10]]]), chunk11);
  const canonicalWithMissedChunk = message(10, "Hello world!");
  const afterMissedChunk = mergeEvents(liveChunks, "s1", [canonicalWithMissedChunk]).get("s1")!;
  expect(
    afterMissedChunk[0]?.kind === "update" ? afterMissedChunk[0].update.content?.text : "",
  ).toBe("Hello world!");
  // Conversely, an overlapping older snapshot captured at chunk 10 ("Hello")
  // must not truncate live chunks 10–11 ("Hello world").
  const afterOlderChunkSnapshot = mergeEvents(liveChunks, "s1", [chunk10]).get("s1")!;
  expect(
    afterOlderChunkSnapshot[0]?.kind === "update"
      ? afterOlderChunkSnapshot[0].update.content?.text
      : "",
  ).toBe("Hello world");
});

test("removedSessionFence scopes entries by dataset and clears live sessions without ping-pong", async () => {
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
    setRemovalFenceDataset("dataset-old");
    markSessionRemoved("sess-shared-id");
    expect(isSessionRemoved("sess-shared-id")).toBe(true);

    // A replacement dataset at the same origin starts with a clean fence.
    setRemovalFenceDataset("dataset-new");
    expect(isSessionRemoved("sess-shared-id")).toBe(false);

    // Switching back restores dataset-old's fence.
    setRemovalFenceDataset("dataset-old");
    expect(isSessionRemoved("sess-shared-id")).toBe(true);

    // Evicting an old entry at capacity does not republish it on a later write.
    fakeStorage.setItem("cowboy:removed-sessions:dataset-old", JSON.stringify(["sess-peer-id"]));
    markSessionRemoved("sess-local-second");
    const stored = JSON.parse(fakeStorage.getItem("cowboy:removed-sessions:dataset-old") ?? "[]") as string[];
    expect(stored).toEqual(["sess-peer-id", "sess-local-second"]);
  } finally {
    setRemovalFenceDataset(undefined);
    globals.localStorage = previousStorage;
  }
});
