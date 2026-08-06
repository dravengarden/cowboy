import { assertEquals } from "jsr:@std/assert";
import {
  deriveTurnStatusKind,
  type TurnStatusSignals,
} from "./turnStatusPolicy.ts";

const settled: TurnStatusSignals = {
  status: "running",
  working: false,
  judging: false,
  awaitingUser: false,
  done: false,
  paused: false,
};

Deno.test("judge progress stays out of the Composer status stack", () => {
  assertEquals(
    deriveTurnStatusKind({
      ...settled,
      judging: true,
      awaitingUser: true,
      done: true,
    }),
    null,
  );
});

Deno.test("settled actionable turn states remain Composer-owned", () => {
  assertEquals(
    deriveTurnStatusKind({ ...settled, awaitingUser: true }),
    "awaiting",
  );
  assertEquals(deriveTurnStatusKind({ ...settled, paused: true }), "paused");
  assertEquals(deriveTurnStatusKind({ ...settled, done: true }), "done");
});
