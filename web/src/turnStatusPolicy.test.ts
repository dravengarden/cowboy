import { assertEquals } from "jsr:@std/assert";
import {
  deriveTurnStatusKind,
  type TurnStatusSignals,
} from "./turnStatusPolicy.ts";

const settled: TurnStatusSignals = {
  offline: false,
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

Deno.test("connection loss still outranks stale judge progress", () => {
  assertEquals(
    deriveTurnStatusKind({ ...settled, offline: true, judging: true }),
    "offline",
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
