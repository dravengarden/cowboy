import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_DIAGNOSTIC_LOG_FILTERS,
  diagnosticKindLabel,
  diagnosticLogUrl,
} from "./diagnosticLogs";

Deno.test("diagnostic logs use bounded server-side filters and cursor pagination", () => {
  assertEquals(
    diagnosticLogUrl({
      ...DEFAULT_DIAGNOSTIC_LOG_FILTERS,
      kind: "cache_anomaly",
      severity: "warning",
      agent: "claude",
      window: "30d",
    }, "next-page"),
    "/api/logs?kind=cache_anomaly&severity=warning&state=all&agent=claude&window=30d&limit=25&cursor=next-page",
  );
});

Deno.test("diagnostic log kinds have concise user-facing labels", () => {
  assertEquals(diagnosticKindLabel("session_error"), "Session");
  assertEquals(diagnosticKindLabel("provider_error"), "Provider");
  assertEquals(diagnosticKindLabel("cache_anomaly"), "Cache");
  assertEquals(diagnosticKindLabel("automation"), "Automation");
});
