import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_DIAGNOSTIC_LOG_FILTERS,
  diagnosticKindLabel,
  diagnosticLogUrl,
  parseDiagnosticLogFilters,
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

Deno.test("diagnostic logs do not poll or render an empty state beside request errors", async () => {
  const source = await Deno.readTextFile(new URL("./UsageLogs.tsx", import.meta.url));
  assertEquals(source.includes("globalThis.setInterval"), false);
  assertEquals(source.includes("!error && !loading && logs.length === 0"), true);
});

Deno.test("critical and error use distinct severity accents across log dots and filters", async () => {
  const source = await Deno.readTextFile(new URL("./UsageLogs.tsx", import.meta.url));
  assertEquals(source.includes('critical: (theme) => theme.palette.mode === "dark" ? "#ff4d6d" : "#c9184a"'), true);
  assertEquals(source.includes('error: (theme) => theme.palette.mode === "dark" ? "#ff8a65" : "#c2410c"'), true);
  assertEquals(source.includes("accent: SEVERITY_OPTIONS.find((option) => option.value === value)?.accent"), true);
  assertEquals(source.includes("bgcolor: SEVERITY_ACCENT[entry.severity]"), true);
});

Deno.test("diagnostic log kinds have concise user-facing labels", () => {
  assertEquals(diagnosticKindLabel("session_error"), "Session");
  assertEquals(diagnosticKindLabel("provider_error"), "Provider");
  assertEquals(diagnosticKindLabel("cache_anomaly"), "Cache");
  assertEquals(diagnosticKindLabel("automation"), "Automation");
});

Deno.test("diagnostic log filters round-trip persisted multi-select and reject unknown values", () => {
  assertEquals(parseDiagnosticLogFilters({
    kinds: ["provider_error", "not-a-kind", "provider_error"],
    severities: ["warning"],
    states: ["failed"],
    agents: ["claude"],
    timeRange: { mode: "relative", amount: 2, unit: "hour" },
  }), {
    kinds: ["provider_error"],
    severities: ["warning"],
    states: ["failed"],
    agents: ["claude"],
    timeRange: { mode: "relative", amount: 2, unit: "hour" },
  });
});

Deno.test("malformed persisted diagnostic filters return safe defaults", () => {
  assertEquals(parseDiagnosticLogFilters({
    kinds: ["provider_error"],
    timeRange: { mode: "absolute", fromMs: 9, toMs: 2 },
  }), {
    ...DEFAULT_DIAGNOSTIC_LOG_FILTERS,
    kinds: ["provider_error"],
  });
});
