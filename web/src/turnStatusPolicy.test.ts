import { assertEquals } from "jsr:@std/assert";
import {
  deriveTurnStatusKind,
  type TurnStatusSignals,
} from "./turnStatusPolicy.ts";

const settled: TurnStatusSignals = {
  status: "running",
  working: false,
  paused: false,
};

Deno.test("manual queue pause remains Composer-owned", () => {
  assertEquals(deriveTurnStatusKind({ ...settled, paused: true }), "paused");
});

Deno.test("crashes stay on the transcript status bar instead of overlay Retry", () => {
  assertEquals(
    deriveTurnStatusKind({ ...settled, status: "crashed" }),
    null,
  );
});
