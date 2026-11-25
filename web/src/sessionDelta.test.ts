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
});

test("authoritativeLinkMetaMap and diffAuthoritativeLinkMeta track trashed sessions and authoritative titles", () => {
  const a = session("a");
  const trashedB: SessionMeta = { ...session("b"), trashed_at_ms: 1_700_000_000_000 };
  const prev = authoritativeLinkMetaMap([a, trashedB], { a: "Old Title" });
  // Authoritative title update + closing a trashed session + removing nothing.
  const next = authoritativeLinkMetaMap(
    [a, { ...trashedB, closing: true }],
    { a: "Committed Title" },
  );
  expect(diffAuthoritativeLinkMeta(prev, next).sort()).toEqual(["a", "b"]);
  expect(diffAuthoritativeLinkMeta(next, next)).toEqual([]);
  // Removing a trashed session is detected too.
  const afterPurge = authoritativeLinkMetaMap([a], { a: "Committed Title" });
  expect(diffAuthoritativeLinkMeta(next, afterPurge)).toEqual(["b"]);
});
