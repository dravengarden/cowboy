import { assertEquals } from "jsr:@std/assert";
import { CRASH_INCIDENT_SEVERITY } from "./observability.ts";

Deno.test("application crashes use critical incident severity", () => {
  assertEquals(CRASH_INCIDENT_SEVERITY, "critical");
});
