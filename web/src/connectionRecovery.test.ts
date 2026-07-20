import { assertEquals } from "jsr:@std/assert";
import { shouldReconnectOnForeground } from "./connectionRecovery.ts";

Deno.test("foreground recovery replaces every non-open socket", () => {
  assertEquals(shouldReconnectOnForeground(undefined, 0, 30_000), true);
  assertEquals(shouldReconnectOnForeground(0, 0, 30_000), true);
  assertEquals(shouldReconnectOnForeground(2, 0, 30_000), true);
  assertEquals(shouldReconnectOnForeground(3, 0, 30_000), true);
});

Deno.test("foreground recovery preserves a fresh open socket", () => {
  assertEquals(shouldReconnectOnForeground(1, 29_999, 30_000), false);
  assertEquals(shouldReconnectOnForeground(1, 30_001, 30_000), true);
});
