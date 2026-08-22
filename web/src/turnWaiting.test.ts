import { assertEquals } from "jsr:@std/assert";
import {
  WAITING_ELAPSED_VISIBLE_SECONDS,
  waitingActivityLabel,
} from "./turnWaiting.ts";

Deno.test("agent wait activity names the provider immediately", () => {
  assertEquals(waitingActivityLabel("Grok", 0), "Waiting for Grok…");
  assertEquals(
    waitingActivityLabel("Grok", WAITING_ELAPSED_VISIBLE_SECONDS - 1),
    "Waiting for Grok…",
  );
});

Deno.test("agent wait activity exposes elapsed seconds after a short silence", () => {
  assertEquals(
    waitingActivityLabel("Grok", WAITING_ELAPSED_VISIBLE_SECONDS),
    "Waiting for Grok · 5s",
  );
  assertEquals(waitingActivityLabel("Codex", 53), "Waiting for Codex · 53s");
});
