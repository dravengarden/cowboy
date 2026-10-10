import { test } from "bun:test";
import { assertEquals, assertNotEquals } from "@std/assert";
import { nestedMarkerColor } from "./nestedMarkerColors.ts";

test("nested shell emoji keep semantic colors across renders", () => {
  assertEquals(nestedMarkerColor("🌺", true), nestedMarkerColor("🌺", true, 57));
  assertEquals(nestedMarkerColor("🌊", false), nestedMarkerColor("🌊", false, 2));
  assertNotEquals(nestedMarkerColor("🌺", true), nestedMarkerColor("🍀", true));
  assertNotEquals(nestedMarkerColor("🔥", false), nestedMarkerColor("🪐", false));
});

test("unknown nested markers retain a deterministic fallback", () => {
  assertEquals(nestedMarkerColor("?", true, 3), nestedMarkerColor("?", true, 3));
  assertNotEquals(nestedMarkerColor("?", true, 3), nestedMarkerColor("?", true, 4));
});
