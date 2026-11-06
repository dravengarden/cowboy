import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { CRASH_INCIDENT_SEVERITY } from "./observability.ts";

test("application crashes use critical incident severity", () => {
  assertEquals(CRASH_INCIDENT_SEVERITY, "critical");
});
