import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import type { QueuedMessage } from "../store.ts";
import {
  adjacentSendOrderBuffer,
  firstLinePreview,
  resolveBufferRow,
  resolveLastSubmittedRow,
} from "./promptBuffer.ts";

const row = (id: string, text: string, cmid?: string): QueuedMessage => ({
  id,
  text,
  attachments: [],
  ...(cmid ? { cmid } : {}),
});

test("resolveBufferRow matches by server id first and falls back to cmid", () => {
  const queue = [row("srv-1", "hello", "c-1"), row("srv-2", "world", "c-2")];
  const byId = resolveBufferRow(
    { kind: "queued", id: "srv-2", origin: "list", entryMode: "normal" },
    queue,
    [],
  );
  assertEquals(byId?.index, 1);
  assertEquals(byId?.row.id, "srv-2");

  // Optimistic id replaced by server id; cmid keeps binding intact.
  const byCmid = resolveBufferRow(
    { kind: "queued", id: "opt-1", cmid: "c-1", origin: "new", entryMode: "insert" },
    queue,
    [],
  );
  assertEquals(byCmid?.index, 0);
  assertEquals(byCmid?.row.id, "srv-1");
});

test("adjacentSendOrderBuffer walks Queue items and places New Prompt at tail", () => {
  const queue = [row("q1", "first"), row("q2", "second")];
  const prevFromNew = adjacentSendOrderBuffer({ kind: "new" }, queue, -1);
  assertEquals(prevFromNew?.kind, "queued");
  if (prevFromNew?.kind === "queued") assertEquals(prevFromNew.id, "q2");

  const prevFromQ2 = adjacentSendOrderBuffer(
    { kind: "queued", id: "q2", origin: "new", entryMode: "normal" },
    queue,
    -1,
  );
  assertEquals(prevFromQ2?.kind, "queued");
  if (prevFromQ2?.kind === "queued") assertEquals(prevFromQ2.id, "q1");

  // At head, stepping backward is a no-op.
  assertEquals(
    adjacentSendOrderBuffer(
      { kind: "queued", id: "q1", origin: "new", entryMode: "normal" },
      queue,
      -1,
    ),
    null,
  );

  // Stepping forward from last queued item lands on New Prompt.
  const nextFromQ2 = adjacentSendOrderBuffer(
    { kind: "queued", id: "q2", origin: "new", entryMode: "normal" },
    queue,
    1,
  );
  assertEquals(nextFromQ2, { kind: "new" });
});

test("resolveLastSubmittedRow matches strictly by cmid and ignores same-text rows", () => {
  const queue = [row("q1", "fix tests", "c-older"), row("q2", "fix tests", "c-latest")];
  const matched = resolveLastSubmittedRow(
    { kind: "queued", cmid: "c-latest", at: 1 },
    queue,
    [],
  );
  assertEquals(matched?.row.id, "q2");

  const gone = resolveLastSubmittedRow(
    { kind: "queued", cmid: "c-already-sent", at: 2 },
    queue,
    [],
  );
  assertEquals(gone, null);
});

test("firstLinePreview collapses whitespace, strips image tokens, and truncates", () => {
  assertEquals(firstLinePreview("  hello\n\nworld  "), "hello world");
  assertEquals(firstLinePreview("![](cowboy-att:img1)"), "(attachment)");
  assertEquals(firstLinePreview("a".repeat(80), 10), "aaaaaaaaa…");
});
