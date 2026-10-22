import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { STALL_TICK_MS, stallDetector } from "./mainThreadStalls.ts";

test("a late visible tick reports its stall; on-time and hidden ticks do not", () => {
  let now = 0;
  let visible = true;
  const tick = stallDetector({ now: () => now, visible: () => visible });
  now += STALL_TICK_MS + 10;
  assertEquals(tick(), null);
  now += STALL_TICK_MS + 240;
  assertEquals(tick(), 240);
  visible = false;
  now += 5_000;
  assertEquals(tick(), null);
  // The first tick after returning still spans the hidden period.
  visible = true;
  now += 5_000;
  assertEquals(tick(), null);
  now += STALL_TICK_MS + 60;
  assertEquals(tick(), 60);
});
