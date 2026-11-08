import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  adjacentIndex,
  adjacentLine,
  reviewDocumentAction,
  userMapsSequence,
} from "./reviewDocumentKeys.ts";

const normal = { keyBuffer: "", operator: false, count: 0, visual: false };
const none = () => false;

test("bracket and g prefixes resolve Review motions", () => {
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "]" }, "c", none), { kind: "hunk", offset: 1 });
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "[" }, "f", none), { kind: "file", offset: -1 });
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "]" }, "u", none), { kind: "unreviewed", offset: 1 });
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "g" }, "t", none), { kind: "tab", offset: 1 });
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "g", count: 3 }, "t", none), { kind: "nthTab", n: 3 });
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "g", count: 2 }, "T", none), { kind: "tab", offset: -2 });
  assertEquals(reviewDocumentAction(normal, "<CR>", none), { kind: "send" });
});

test("Vim keeps everything else, pending operators and Visual motions", () => {
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "]" }, "]", none), null);
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "g" }, "g", none), null);
  assertEquals(reviewDocumentAction(normal, "j", none), null);
  assertEquals(reviewDocumentAction({ ...normal, operator: true }, "<CR>", none), null);
  assertEquals(reviewDocumentAction({ ...normal, visual: true, keyBuffer: "]" }, "c", none), null);
  assertEquals(reviewDocumentAction({ ...normal, visual: true }, "<CR>", none), { kind: "send" });
});

test("a user mapping of the same keys wins", () => {
  const mappings = [{ lhs: "]c", rhs: "zz", mode: "normal" as const, recursive: false }];
  const mapped = (sequence: string, visual: boolean) => userMapsSequence(mappings, sequence, visual);
  assertEquals(reviewDocumentAction({ ...normal, keyBuffer: "]" }, "c", mapped), null);
  assertEquals(userMapsSequence(mappings, "]c", true), false);
});

test("adjacent anchors stop at the ends", () => {
  assertEquals(adjacentLine([3, 10, 20], 3, 1), 10);
  assertEquals(adjacentLine([3, 10, 20], 3, -1), undefined);
  assertEquals(adjacentLine([3, 10, 20], 15, -1), 10);
  assertEquals(adjacentIndex(4, 1, 1), 2);
  assertEquals(adjacentIndex(4, 3, 1), undefined);
  assertEquals(adjacentIndex(4, -1, 1), 0);
  assertEquals(adjacentIndex(4, -1, -1), 3);
  assertEquals(adjacentIndex(4, 0, 1, (i) => i === 3), 3);
});
