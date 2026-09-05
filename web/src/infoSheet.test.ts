import { assertEquals } from "jsr:@std/assert";

const infoSheetSource = await Deno.readTextFile(
  new URL("./InfoSheet.tsx", import.meta.url),
);
const detailsSource = await Deno.readTextFile(
  new URL("./ProviderUsageActivityDetails.tsx", import.meta.url),
);
const timeRangeSource = await Deno.readTextFile(
  new URL("./ObservabilityFilters.tsx", import.meta.url),
);
const usageLogsSource = await Deno.readTextFile(
  new URL("./UsageLogs.tsx", import.meta.url),
);
const sheetSource = await Deno.readTextFile(
  new URL("./Sheet.tsx", import.meta.url),
);

Deno.test("closed activity renderer exposes windows and key diagnostics", () => {
  assertEquals(
    detailsSource.includes("function ProviderUsageActivityDetails"),
    true,
  );
  assertEquals(detailsSource.includes("TimeRangeButton"), true);
  assertEquals(detailsSource.includes("timeRangeQuery"), true);
  assertEquals(detailsSource.includes("Blocking errors"), true);
  assertEquals(detailsSource.includes("Est. spend"), true);
  assertEquals(detailsSource.includes("Cache hit rate"), true);
  assertEquals(detailsSource.includes("usage.provider"), true);
  assertEquals(detailsSource.includes("DeepSeek"), false);
});

Deno.test("nested observability sheets portal their scrims above the iOS safe area", () => {
  assertEquals(sheetSource.includes("createPortal(sheet"), true);
  assertEquals(
    timeRangeSource.includes('portal\n        title="Time range"'),
    true,
  );
  assertEquals(
    usageLogsSource.includes('portal\n        title="Filter diagnostic logs"'),
    true,
  );
});

Deno.test("desktop Info uses independent columns and compact metric tiles", () => {
  assertEquals(infoSheetSource.includes('gridRow: "1 / span 4"'), false);
  assertEquals(infoSheetSource.includes("repeat(2, minmax(0, 1fr))"), true);
  assertEquals(infoSheetSource.includes('bgcolor: "action.hover"'), true);
});

Deno.test("activity usage controls stay readable on tablet and desktop widths", () => {
  assertEquals(
    detailsSource.includes('xs: "repeat(2, minmax(0, 1fr))"'),
    true,
  );
  assertEquals(
    detailsSource.includes('sm: "repeat(4, minmax(0, 1fr))"'),
    true,
  );
});

Deno.test("diagnostic detail uses compact scan lines instead of nested field cards", () => {
  assertEquals(
    usageLogsSource.includes(
      '"minmax(96px, 0.36fr) minmax(0, 1fr) 24px"',
    ),
    true,
  );
  assertEquals(
    usageLogsSource.includes("data-diagnostic-detail-section"),
    true,
  );
  assertEquals(usageLogsSource.includes("data-diagnostic-detail-field"), true);
  assertEquals(usageLogsSource.includes('"@media (max-width: 720px)"'), true);
  assertEquals(
    usageLogsSource.includes('gridTemplateColumns: "84px minmax(0, 1fr) 24px"'),
    true,
  );
  assertEquals(usageLogsSource.includes('textAlign: "right"'), false);
  assertEquals(usageLogsSource.includes('borderColor: "divider"'), true);
});
