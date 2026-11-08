import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { codeReadEndpoint } from "./reviewReadTelemetry.ts";

test("review read telemetry names the resource without ids or queries", () => {
  assertEquals(
    codeReadEndpoint("/api/code/sessions/sess-1/diff?path=secret.ts&scope=x"),
    "diff",
  );
  assertEquals(codeReadEndpoint("/api/code/sessions/a%2Fb/commit-diff"), "commit-diff");
  assertEquals(codeReadEndpoint("/api/code/sessions/s/changes"), "changes");
  assertEquals(codeReadEndpoint("/api/other"), "other");
});
