import {
  assertEquals,
  assertRejects,
  assertStringIncludes,
} from "jsr:@std/assert";
import { adminApi, type PluginRelease } from "./adminApi.ts";

const release: PluginRelease = {
  plugin_id: "victoria",
  plugin_version: "1.1.0",
  plugin_kind: "telemetry_backend",
  package_digest: `sha256:${"a".repeat(64)}`,
  artifact_digest: `sha256:${"b".repeat(64)}`,
  publisher: "fixture",
  release_state: "ready",
  supported_platforms: [{ os: "linux", architecture: "x86_64" }],
};

Deno.test("admin install uses the registered generic Plugin route and exact release", async () => {
  const original = globalThis.fetch;
  const calls: Array<[string, RequestInit | undefined]> = [];
  globalThis.fetch = (input, init) => {
    calls.push([String(input), init]);
    return Promise.resolve(new Response(null, { status: 204 }));
  };
  try {
    await adminApi.installPlugin("machine-test", release);
    assertEquals(calls.length, 1);
    const [path, init] = calls[0];
    assertEquals(path, "/api/machines/machine-test/plugins/victoria");
    assertEquals(init?.method, "POST");
    assertEquals(init?.credentials, "same-origin");
    assertEquals(init?.cache, "no-store");
    assertEquals(JSON.parse(String(init?.body)), {
      version: release.plugin_version,
      digest: release.artifact_digest,
    });
    const routes = await Deno.readTextFile(
      new URL("../../../src/server.rs", import.meta.url),
    );
    assertStringIncludes(routes, '"/api/machines/{id}/plugins/{provider_id}"');
  } finally {
    globalThis.fetch = original;
  }
});

Deno.test("admin install refuses unbound releases without sending a request", async () => {
  const original = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = () => {
    calls += 1;
    return Promise.resolve(new Response(null, { status: 204 }));
  };
  try {
    for (
      const plugin of [{ ...release, artifact_digest: null }, {
        ...release,
        release_state: "unbound",
      }]
    ) {
      await assertRejects(
        () => adminApi.installPlugin("machine-test", plugin),
        Error,
        "signed Plugin release",
      );
    }
    assertEquals(calls, 0);
  } finally {
    globalThis.fetch = original;
  }
});
