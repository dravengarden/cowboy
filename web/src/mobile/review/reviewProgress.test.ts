import { assertEquals } from "jsr:@std/assert";
import { revisionMatches } from "./reviewProgress.ts";

Deno.test("reviewed state requires the exact diff revision", () => {
  const progress = { "unstaged\u0000src/a.ts": "revision-a" };
  assertEquals(
    revisionMatches(progress, "unstaged\u0000src/a.ts", "revision-a"),
    true,
  );
  assertEquals(
    revisionMatches(progress, "unstaged\u0000src/a.ts", "revision-b"),
    false,
  );
  assertEquals(
    revisionMatches(progress, "staged\u0000src/a.ts", "revision-a"),
    false,
  );
});
