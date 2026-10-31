import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { lastRowBefore, requestSessionReveal, takeSessionReveal } from "./sessionReveal.ts";

test("the boundary is exclusive and ignores optimistic rows", () => {
  assertEquals(lastRowBefore(["3", "7", "12", "optimistic-1"], 12), "7");
  assertEquals(lastRowBefore(["12", "20"], 12), undefined);
});

test("a reveal is consumed once", () => {
  requestSessionReveal("s", undefined, 9);
  assertEquals(takeSessionReveal("s"), 9);
  assertEquals(takeSessionReveal("s"), undefined);
});
