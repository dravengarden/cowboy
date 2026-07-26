import { assertEquals } from "jsr:@std/assert";
import { hapticStyleForIntent } from "./hapticIntent.ts";

Deno.test("Cowboy haptic intents preserve the product strength hierarchy", () => {
  assertEquals(hapticStyleForIntent("navigation"), "selection");
  assertEquals(hapticStyleForIntent("magnetic"), "selection");
  assertEquals(hapticStyleForIntent("confirmation"), "medium");
  assertEquals(hapticStyleForIntent("important"), "heavy");
});
