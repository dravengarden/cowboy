import { assertEquals } from "jsr:@std/assert";
import { outputPrefersHorizontalScroll } from "./outputLayout.ts";

Deno.test("terminal tables preserve their horizontal columns", () => {
  assertEquals(outputPrefersHorizontalScroll(`
Column     | Type      | Nullable
-----------+-----------+----------
session_id | text      | not null
seq        | bigint    | not null
`), true);
  assertEquals(outputPrefersHorizontalScroll("NAME\tSTATUS\tAGE\napi\tRunning\t3d"), true);
  assertEquals(outputPrefersHorizontalScroll("NAME     STATUS    AGE\napi      Running   3d\nworker   Ready     2d"), true);
});

Deno.test("ordinary terminal prose remains wrapped", () => {
  assertEquals(outputPrefersHorizontalScroll("Build completed successfully.\nNo warnings were reported."), false);
  assertEquals(outputPrefersHorizontalScroll("one short line"), false);
});
