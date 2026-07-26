import { assertEquals } from "jsr:@std/assert";
import {
  diffCacheSizeForTest,
  invalidateDiffCache,
  loadCodeDiff,
} from "./diffCache.ts";

Deno.test("diff cache deduplicates adjacent prefetch and foreground loads", async () => {
  const originalFetch = globalThis.fetch;
  let requests = 0;
  globalThis.fetch = (() => {
    requests += 1;
    return Promise.resolve(
      new Response(
        JSON.stringify({
          apiVersion: 1,
          path: "src/a.ts",
          text: "@@ -1 +1 @@\n-old\n+new\n",
          added: 1,
          removed: 1,
          truncated: false,
        }),
        { status: 200 },
      ),
    );
  }) as typeof fetch;
  try {
    invalidateDiffCache();
    const first = loadCodeDiff("session", "src/a.ts", 6, true, "unstaged");
    const second = loadCodeDiff("session", "src/a.ts", 6, true, "unstaged");
    assertEquals(await first, await second);
    assertEquals(requests, 1);
    assertEquals(diffCacheSizeForTest(), 1);
    for (let index = 0; index < 12; index += 1) {
      await loadCodeDiff(
        "session",
        `src/${index}.ts`,
        6,
        true,
        "unstaged",
      );
    }
    assertEquals(requests, 13);
    assertEquals(diffCacheSizeForTest(), 12);
  } finally {
    invalidateDiffCache();
    globalThis.fetch = originalFetch;
  }
});
