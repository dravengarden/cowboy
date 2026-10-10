import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { shouldApplyHydratedConfigOptions } from "./configOptionsHydration";

test("hydration may seed config options when no live update raced it", () => {
  assertEquals(shouldApplyHydratedConfigOptions(3, 3), true);
});

test("hydration cannot overwrite a newer live config update", () => {
  assertEquals(shouldApplyHydratedConfigOptions(3, 4), false);
});
