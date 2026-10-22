import { test } from "bun:test";
import { assert, assertEquals, assertStrictEquals } from "@std/assert";
import type { SessionMeta } from "./protocol.ts";
import { jsonValueEqual, reuseUnchangedSessions } from "./sessionIdentity.ts";

function session(id: string, extra: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id,
    provider: "codex",
    cwd: "/w",
    title: id,
    status: "running",
    ...extra,
  };
}

test("an unchanged frame keeps the previous array and every row identity", () => {
  const previous = [
    session("a"),
    session("b", { context: { used: 1, size: 2 } } as Partial<SessionMeta>),
  ];
  const next = JSON.parse(JSON.stringify(previous)) as SessionMeta[];
  assertStrictEquals(reuseUnchangedSessions(previous, next), previous);
});

test("only the changed session gets a new identity", () => {
  const previous = [session("a"), session("b"), session("c")];
  const next = [session("a"), session("b", { status: "busy" }), session("c")];
  const out = reuseUnchangedSessions(previous, next);
  assert(out !== previous);
  assertStrictEquals(out[0], previous[0]);
  assertStrictEquals(out[1], next[1]);
  assertStrictEquals(out[2], previous[2]);
});

test("a reorder, removal or addition yields a new array of reused rows", () => {
  const previous = [session("a"), session("b")];
  const reordered = reuseUnchangedSessions(previous, [
    session("b"),
    session("a"),
  ]);
  assertEquals(reordered.map((s) => s.id), ["b", "a"]);
  assertStrictEquals(reordered[0], previous[1]);
  assertEquals(reuseUnchangedSessions(previous, [session("a")]).length, 1);
  assertEquals(
    reuseUnchangedSessions(previous, [...previous, session("c")]).length,
    3,
  );
});

test("structural equality distinguishes missing keys, arrays and nested values", () => {
  assert(jsonValueEqual({ a: [1, { b: null }] }, { a: [1, { b: null }] }));
  assert(!jsonValueEqual({ a: 1 }, { a: 1, b: undefined }));
  assert(!jsonValueEqual([1], { 0: 1 }));
  assert(!jsonValueEqual({ a: { b: 1 } }, { a: { b: 2 } }));
});
