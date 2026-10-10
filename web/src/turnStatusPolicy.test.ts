import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  deriveTurnStatusKind,
  type TurnStatusSignals,
} from "./turnStatusPolicy.ts";

const settled: TurnStatusSignals = {
  status: "running",
  working: false,
  paused: false,
};

test("manual queue pause remains Composer-owned", () => {
  assertEquals(deriveTurnStatusKind({ ...settled, paused: true }), "paused");
});

test("crashes stay on the transcript status bar instead of overlay Retry", () => {
  assertEquals(
    deriveTurnStatusKind({ ...settled, status: "crashed" }),
    null,
  );
});
