import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import {
  CapacityMeter,
  capacityPercent,
  MetricDistribution,
  metricPercentLabel,
} from "./InfoMetricVisuals";
import { ClientMetricsSummary } from "./InfoRuntimeMetrics";
import { relativeUpdateTime } from "./usageLimits";

test("capacity distinguishes missing, zero, tiny and overcommitted measurements", () => {
  assertEquals(capacityPercent(undefined, 100), undefined);
  assertEquals(capacityPercent(0, 0), undefined);
  assertEquals(capacityPercent(Number.NaN, 100), undefined);
  assertEquals(capacityPercent(-1, 100), undefined);
  assertEquals(capacityPercent(0, 100), 0);
  assertEquals(capacityPercent(120, 100), 120);
  assertEquals(metricPercentLabel(0.4), "<1%");
  assertEquals(metricPercentLabel(99.6), ">99%");
  const exceeded = renderToStaticMarkup(
    createElement(CapacityMeter, { label: "Workers", value: 12, total: 10 }),
  );
  assert(exceeded.includes('aria-valuenow="100"'));
  assert(exceeded.includes("120%"));
  assert(exceeded.includes("12 of 10"));
  const unknown = renderToStaticMarkup(
    createElement(CapacityMeter, {
      label: "Storage",
      value: undefined,
      total: 100,
    }),
  );
  assert(!unknown.includes('role="meter"'));
  assert(unknown.includes("Not reported"));
});

test("client storage does not invent capacity or heap readings unavailable on WebKit", () => {
  const markup = renderToStaticMarkup(
    createElement(ClientMetricsSummary, {
      metrics: {
        bytes: 1024,
        entries: 19,
        drafts: 1,
        surface: "Native app",
        serviceWorker: "Unsupported",
      },
    }),
  );
  assert(markup.includes("App storage used"));
  assert(markup.includes("Not reported"));
  assert(!markup.includes('role="meter"'));
  assert(!markup.includes("JS heap"));
  assert(markup.includes("Native app"));
  assert(markup.includes("Not available"));
  const known = renderToStaticMarkup(
    createElement(ClientMetricsSummary, {
      metrics: {
        bytes: 1024,
        entries: 19,
        drafts: 1,
        surface: "Browser tab",
        serviceWorker: "Active",
        storageUsageBytes: 1024,
        storageQuotaBytes: 102400,
        jsHeapUsedBytes: 1024,
        jsHeapLimitBytes: 2048,
      },
    }),
  );
  assert(known.includes("up to 100.0 KB"));
  assert(known.includes("not total device disk space"));
  assert(known.includes("JS heap"));
});

test("an empty record population is visibly zero rather than missing data", () => {
  const empty = renderToStaticMarkup(createElement(MetricDistribution, {
    label: "Session records",
    segments: [{ label: "live", value: 0, tone: "info" }, { label: "deleted", value: 0, tone: "neutral" }],
  }));
  assert(empty.includes("0 live"));
  assert(empty.includes("0 deleted"));
  assert(!empty.includes('role="img"'));
});

test("usage timestamps stay readable from seconds to old cached observations", () => {
  const now = 100 * 86_400_000;
  assertEquals(relativeUpdateTime(0, now), "Not updated");
  assertEquals(relativeUpdateTime(Number.NaN, now), "Not updated");
  assertEquals(relativeUpdateTime(now + 1000, now), "Just now");
  assertEquals(relativeUpdateTime(now - 60_000, now), "1m ago");
  assertEquals(relativeUpdateTime(now - 3_600_000, now), "1h ago");
  assertEquals(relativeUpdateTime(now - 66_106 * 60_000, now), "45d ago");
});
