import { assertEquals } from "jsr:@std/assert";

const infoSheetSource = await Deno.readTextFile(
  new URL("./InfoSheet.tsx", import.meta.url),
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
  assertEquals(infoSheetSource.includes("cacheKeepaliveRequests"), true);
  assertEquals(infoSheetSource.includes("Protection spend"), true);
  assertEquals(infoSheetSource.includes("Verified hit rate"), true);
  assertEquals(infoSheetSource.includes("not included in agent spend"), true);
  assertEquals(infoSheetSource.includes("Schema v3+"), true);
  assertEquals(infoSheetSource.includes("DEEPSEEK_CACHE_MIN_HIT_LABEL"), true);
  assertEquals(infoSheetSource.includes("animateOnOpen"), true);
});
