import { assertEquals } from "jsr:@std/assert";
import { sessionJumpLabels } from "./sessionJump.ts";

Deno.test("session jump labels start on the home row in displayed order", () => {
  const labels = sessionJumpLabels(["one", "two", "three"]);
  assertEquals([...labels.values()], ["a", "s", "d"]);
});

Deno.test("session jump labels stop at the alphabet", () => {
  const sessions = Array.from({ length: 30 }, (_, index) => String(index));
  const labels = sessionJumpLabels(sessions);
  assertEquals(labels.size, 26);
  assertEquals(new Set(labels.values()).size, 26);
  assertEquals(labels.get("29"), undefined);
});
