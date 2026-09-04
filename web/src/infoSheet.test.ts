import { assertEquals } from "jsr:@std/assert";

const infoSheetSource = await Deno.readTextFile(
  new URL("./InfoSheet.tsx", import.meta.url),
);
const detailsSource = await Deno.readTextFile(
  new URL(
    "../../plugins/claude-deepseek/ui/DeepSeekDetails.tsx",
    import.meta.url,
  ),
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

Deno.test("DeepSeek usage exposes diagnostic time and error controls", () => {
  assertEquals(detailsSource.includes("TimeRangeButton"), true);
  assertEquals(detailsSource.includes("MultiSelectChipGroup"), true);
  assertEquals(detailsSource.includes("Blocking errors"), true);
  assertEquals(detailsSource.includes("of requests"), true);
  assertEquals(detailsSource.includes("Retryable provider failures"), true);
  assertEquals(detailsSource.includes("Clear selections"), true);
  assertEquals(detailsSource.includes("resetFilters"), true);
  assertEquals(detailsSource.includes("Cache miss rate"), true);
  assertEquals(detailsSource.includes("usageCacheOptionName"), true);
  assertEquals(detailsSource.includes("usageCacheIntervalLabel"), true);
  assertEquals(detailsSource.includes("cacheKeepaliveRequests"), true);
  assertEquals(detailsSource.includes("Protection spend"), true);
  assertEquals(detailsSource.includes("Verified hit rate"), true);
  assertEquals(detailsSource.includes("not included in agent spend"), true);
  assertEquals(detailsSource.includes("Schema v3+"), true);
  assertEquals(detailsSource.includes("usageCacheMinHitLabel"), true);
});

Deno.test("nested observability sheets portal their scrims above the iOS safe area", () => {
  assertEquals(sheetSource.includes("createPortal(sheet"), true);
  assertEquals(
    timeRangeSource.includes('portal\n        title="Time range"'),
    true,
  );
  assertEquals(
    detailsSource.includes('portal\n        title="Filter DeepSeek usage"'),
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

Deno.test("DeepSeek usage controls stay readable on tablet and desktop widths", () => {
  assertEquals(
    detailsSource.includes('spacing={0.75} sx={{ width: "100%", maxWidth: 560 }}'),
    true,
  );
});

Deno.test("diagnostic detail uses compact scan lines instead of nested field cards", () => {
  assertEquals(
    usageLogsSource.includes(
      'gridTemplateColumns: "minmax(96px, 0.36fr) minmax(0, 1fr) 24px"',
    ),
    true,
  );
  assertEquals(usageLogsSource.includes("data-diagnostic-detail-section"), true);
  assertEquals(usageLogsSource.includes("data-diagnostic-detail-field"), true);
  assertEquals(usageLogsSource.includes('"@media (max-width: 720px)"'), true);
  assertEquals(usageLogsSource.includes('gridTemplateColumns: "84px minmax(0, 1fr) 24px"'), true);
  assertEquals(usageLogsSource.includes('textAlign: "right"'), false);
  assertEquals(usageLogsSource.includes('borderColor: "divider"'), true);
});
