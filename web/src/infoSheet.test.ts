import { assertEquals } from "jsr:@std/assert";

const infoSheetSource = await Deno.readTextFile(
  new URL("./InfoSheet.tsx", import.meta.url),
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
  assertEquals(infoSheetSource.includes("TimeRangeButton"), true);
  assertEquals(infoSheetSource.includes("MultiSelectChipGroup"), true);
  assertEquals(infoSheetSource.includes("Blocking errors"), true);
  assertEquals(infoSheetSource.includes("of requests"), true);
  assertEquals(infoSheetSource.includes("Retryable provider failures"), true);
  assertEquals(infoSheetSource.includes("Clear selections"), true);
  assertEquals(infoSheetSource.includes("resetFilters"), true);
  assertEquals(infoSheetSource.includes("Cache miss rate"), true);
  assertEquals(infoSheetSource.includes("Cache protection"), true);
  assertEquals(infoSheetSource.includes("DEEPSEEK_CACHE_BASE_INTERVAL_LABEL"), true);
  assertEquals(infoSheetSource.includes("cacheKeepaliveRequests"), true);
  assertEquals(infoSheetSource.includes("Protection spend"), true);
  assertEquals(infoSheetSource.includes("Verified hit rate"), true);
  assertEquals(infoSheetSource.includes("not included in agent spend"), true);
  assertEquals(infoSheetSource.includes("Schema v3+"), true);
  assertEquals(infoSheetSource.includes("DEEPSEEK_CACHE_MIN_HIT_LABEL"), true);
});

Deno.test("nested observability sheets portal their scrims above the iOS safe area", () => {
  assertEquals(sheetSource.includes("createPortal(sheet"), true);
  assertEquals(
    timeRangeSource.includes('portal\n        title="Time range"'),
    true,
  );
  assertEquals(
    infoSheetSource.includes('portal\n        title="Filter DeepSeek usage"'),
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
    infoSheetSource.includes('spacing={0.75} sx={{ width: "100%", maxWidth: 560 }}'),
    true,
  );
});

Deno.test("diagnostic detail keeps labels and values in a bounded grid", () => {
  assertEquals(
    usageLogsSource.includes('gridTemplateColumns: "repeat(2, minmax(0, 1fr))"'),
    true,
  );
  assertEquals(usageLogsSource.includes('"@media (max-width: 720px)"'), true);
  assertEquals(usageLogsSource.includes('textAlign: "right"'), false);
  assertEquals(usageLogsSource.includes('borderColor: "divider"'), true);
});
