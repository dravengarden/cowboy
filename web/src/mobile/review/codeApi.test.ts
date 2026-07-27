import { assertEquals } from "jsr:@std/assert";
import {
  closeCodeBuffer,
  fetchCodeFilePage,
  fetchCodeLanguage,
  fetchCodeManifest,
  fetchCodeSearch,
  fetchCodeTree,
  openCodeBuffer,
} from "./codeApi.ts";

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

Deno.test("language intelligence stays inside the stable Code data plane", async () => {
  const originalFetch = globalThis.fetch;
  let requested = "";
  globalThis.fetch = ((input: string | URL | Request) => {
    requested = String(input);
    return Promise.resolve(
      new Response(
        JSON.stringify({
          apiVersion: 1,
          path: "src/main.rs",
          version: [],
          diagnostics: [],
          inlayHints: [],
          semanticTokens: [],
        }),
        { status: 200 },
      ),
    );
  }) as typeof fetch;
  try {
    await fetchCodeLanguage("session/id", "src/main.rs");
    assertEquals(
      requested,
      "/api/code/sessions/session%2Fid/language?path=src%2Fmain.rs",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

Deno.test("search stays inside the stable Code data plane", async () => {
  const originalFetch = globalThis.fetch;
  let requested = "";
  globalThis.fetch = ((input: string | URL | Request) => {
    requested = String(input);
    return Promise.resolve(
      new Response(
        JSON.stringify({ apiVersion: 1, files: ["src/main.rs"] }),
        { status: 200 },
      ),
    );
  }) as typeof fetch;
  try {
    await fetchCodeSearch("session/id", "main rs");
    assertEquals(
      requested,
      "/api/code/sessions/session%2Fid/search?q=main+rs&limit=50",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

Deno.test("manifest stays inside the stable Code data plane", async () => {
  const originalFetch = globalThis.fetch;
  let requested = "";
  globalThis.fetch = ((input: string | URL | Request) => {
    requested = String(input);
    return Promise.resolve(
      new Response(
        JSON.stringify({
          apiVersion: 1,
          provider: "local",
          revision: "revision",
        }),
        { status: 200 },
      ),
    );
  }) as typeof fetch;
  try {
    await fetchCodeManifest("session/id");
    assertEquals(
      requested,
      "/api/code/sessions/session%2Fid/manifest",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

Deno.test("file continuation keeps the path and opaque cursor together", async () => {
  const originalFetch = globalThis.fetch;
  let requested = "";
  globalThis.fetch = ((input: string | URL | Request) => {
    requested = String(input);
    return Promise.resolve(
      new Response(
        JSON.stringify({
          apiVersion: 1,
          path: "src/large.ts",
          revision: "revision",
          text: "next\n",
          size: 1_000_000,
          truncated: false,
        }),
        { status: 200 },
      ),
    );
  }) as typeof fetch;
  try {
    await fetchCodeFilePage(
      "session",
      "src/large.ts",
      "revision:262144",
    );
    assertEquals(
      requested,
      "/api/code/sessions/session/file?path=src%2Flarge.ts&cursor=revision%3A262144",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

Deno.test("buffer leases use an idempotent stable Code contract", async () => {
  const originalFetch = globalThis.fetch;
  const requests: Array<{ url: string; method?: string; body?: string }> = [];
  globalThis.fetch = ((input: string | URL | Request, init?: RequestInit) => {
    requests.push({
      url: String(input),
      method: init?.method,
      body: typeof init?.body === "string" ? init.body : undefined,
    });
    return Promise.resolve(
      new Response(
        JSON.stringify({
          apiVersion: 1,
          path: "src/main.rs",
          leases: init?.method === "PUT" ? 1 : 0,
        }),
        { status: 200 },
      ),
    );
  }) as typeof fetch;
  try {
    await openCodeBuffer("session/id", "src/main.rs", "tab-1");
    await closeCodeBuffer("session/id", "src/main.rs", "tab-1");
    assertEquals(requests, [
      {
        url: "/api/code/sessions/session%2Fid/buffer",
        method: "PUT",
        body: '{"path":"src/main.rs","leaseId":"tab-1"}',
      },
      {
        url: "/api/code/sessions/session%2Fid/buffer",
        method: "DELETE",
        body: '{"path":"src/main.rs","leaseId":"tab-1"}',
      },
    ]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
