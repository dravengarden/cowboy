import { assertEquals } from "jsr:@std/assert";
import { shouldApplyHydratedConfigOptions } from "./configOptionsHydration";

Deno.test("hydration may seed config options when no live update raced it", () => {
  assertEquals(shouldApplyHydratedConfigOptions(3, 3), true);
});

Deno.test("hydration cannot overwrite a newer live config update", () => {
  assertEquals(shouldApplyHydratedConfigOptions(3, 4), false);
});
