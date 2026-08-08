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
        kinds: ["cache_anomaly", "provider_error"],
        severities: ["warning", "error"],
        agents: ["claude"],
        timeRange: { mode: "relative", amount: 30, unit: "day" },
      }, "next-page", 40 * 86_400_000),
      `/api/logs?limit=25&kind=cache_anomaly%2Cprovider_error&severity=warning%2Cerror&agent=claude&from_ms=${String(10 * 86_400_000)}&to_ms=${String(40 * 86_400_000)}&cursor=next-page`,
  );
});

Deno.test("diagnostic logs default to serious events without narrowing kind or runtime", () => {
  assertEquals(DEFAULT_DIAGNOSTIC_LOG_FILTERS.severities, ["critical", "error"]);
  assertEquals(DEFAULT_DIAGNOSTIC_LOG_FILTERS.kinds, []);
  assertEquals(DEFAULT_DIAGNOSTIC_LOG_FILTERS.agents, []);
});

Deno.test("diagnostic log kinds have concise user-facing labels", () => {
  assertEquals(diagnosticKindLabel("session_error"), "Session");
  assertEquals(diagnosticKindLabel("provider_error"), "Provider");
  assertEquals(diagnosticKindLabel("cache_anomaly"), "Cache");
  assertEquals(diagnosticKindLabel("automation"), "Automation");
});
