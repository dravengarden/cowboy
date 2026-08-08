import { assertEquals } from "jsr:@std/assert";

const infoSheetSource = await Deno.readTextFile(
  new URL("./InfoSheet.tsx", import.meta.url),
);

Deno.test("DeepSeek usage exposes short rolling windows", () => {
  assertEquals(
    infoSheetSource.includes(
      '"1h",\n  "2h",\n  "4h",\n  "6h",\n  "8h",\n  "12h",\n  "24h",',
    ),
    true,
  );
  assertEquals(infoSheetSource.includes("Error rate"), true);
  assertEquals(infoSheetSource.includes("Cache miss rate"), true);
});
