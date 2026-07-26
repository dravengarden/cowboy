import { assertEquals } from "jsr:@std/assert";
import { fetchCodeTree } from "./codeApi.ts";

Deno.test("tree requests preserve paths and explicit refresh bypasses caches", async () => {
  const originalFetch = globalThis.fetch;
  const requests: Array<{ url: string; cache?: RequestCache }> = [];
  globalThis.fetch = ((input: string | URL | Request, init?: RequestInit) => {
    requests.push({ url: String(input), cache: init?.cache });
    return Promise.resolve(
      new Response(
        JSON.stringify({
          apiVersion: 1,
          path: "src/mobile",
          revision: "revision",
          entries: [],
          truncated: false,
        }),
        { status: 200 },
      ),
    );
  }) as typeof fetch;
  try {
    await fetchCodeTree("session/id", "src/mobile");
    await fetchCodeTree("session/id", "src/mobile", undefined, true);
    assertEquals(requests, [
      {
        url:
          "/api/code/sessions/session%2Fid/tree?path=src%2Fmobile",
        cache: "default",
      },
      {
        url:
          "/api/code/sessions/session%2Fid/tree?path=src%2Fmobile",
        cache: "reload",
      },
    ]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
