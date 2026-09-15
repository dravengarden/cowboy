import { assertEquals, assertRejects } from "jsr:@std/assert";
import type { ProviderUiManifest } from "@cowboy/provider-ui";
import { providerUiManifestFixture } from "./providerUiContract.fixture.ts";
import {
  loadProviderCatalog,
  resetProviderCatalog,
} from "./providerCatalogRegistry.ts";
import { providerUsage, type UsageSnapshot } from "./usageLimits.ts";
import { readUsage, refreshSessionUsage, refreshUsage } from "./usageApi.ts";

const digest = `sha256:${"1".repeat(64)}`;

function entry(manifest: ProviderUiManifest, artifact = digest) {
  return {
    provider_id: manifest.id,
    provider_version: manifest.version,
    package_digest: artifact,
    artifact_digest: artifact,
    authentication_scope: "none-v1",
    release_state: "ready",
    publisher: manifest.publisher,
    contract_fingerprint: artifact,
    supported_platforms: [{ os: "linux", architecture: "x86_64" }],
    manifest,
  };
}

function usageSnapshot(account: string): UsageSnapshot {
  return {
    refreshed_at_ms: 1,
    next_refresh_at_ms: 2,
    refresh_interval_ms: 1,
    providers: [{
      provider: account,
      status: "available",
      source: "fixture",
      observed_at_ms: 1,
    }],
  };
}

Deno.test("session refresh and displayed usage share the declared account identity", async () => {
  const providers = [
    "codex",
    "claude-code",
    "grok",
    "gemini",
    "claude-deepseek",
    "codex-deepseek",
  ];
  const manifests = providers.map((id) => {
    const declared = JSON.parse(
      Deno.readTextFileSync(
        new URL(`../../plugins/${id}/provider.json`, import.meta.url),
      ),
    );
    const manifest = providerUiManifestFixture();
    manifest.id = id;
    manifest.host.account_usage = declared.host.account_usage;
    return manifest;
  });
  const future = providerUiManifestFixture();
  future.id = "future-agent";
  future.host.account_usage = { provider: "future-account" };
  manifests.push(future);
  const accounts = new Set(
    manifests.map((m) => m.host.account_usage!.provider),
  );
  const previousFetch = globalThis.fetch;
  const calls: string[] = [];
  globalThis.fetch = (input, init) => {
    const url = String(input);
    if (url === "/api/plugins") {
      return Promise.resolve(
        Response.json({
          providers: manifests.map((m) => entry(m)),
          authentications: [],
          authentication_executors: [],
          platform: { hosts: [] },
        }),
      );
    }
    calls.push(url);
    const account = url.slice("/api/usage/".length);
    assertEquals(init?.method, "POST");
    // Match UsageService::refresh_provider: only declared account keys exist.
    return Promise.resolve(
      accounts.has(account)
        ? Response.json(usageSnapshot(account))
        : new Response("unknown usage provider", { status: 400 }),
    );
  };
  try {
    resetProviderCatalog();
    await loadProviderCatalog();
    // Reproduce the former request, including the error text that was discarded.
    await assertRejects(
      () => refreshUsage("codex"),
      Error,
      "unknown usage provider",
    );
    for (const manifest of manifests) {
      const snapshot = await refreshSessionUsage(
        manifest.id,
        manifest.version,
        digest,
      );
      const account = manifest.host.account_usage!.provider;
      assertEquals(calls.at(-1), `/api/usage/${account}`);
      assertEquals(
        providerUsage(snapshot, manifest.id, manifest.version, digest)
          ?.provider,
        account,
      );
    }
  } finally {
    globalThis.fetch = previousFetch;
    resetProviderCatalog();
  }
});

Deno.test("session refresh remains pinned to its exact contract and never guesses an account", async () => {
  const installed = providerUiManifestFixture();
  installed.id = "example";
  installed.host.account_usage = { provider: "original-account" };
  const latest = structuredClone(installed);
  latest.version = "9.0.0";
  latest.host.account_usage = { provider: "new-account" };
  const unsupported = providerUiManifestFixture();
  unsupported.id = "without-usage";
  delete unsupported.host.account_usage;
  const previousFetch = globalThis.fetch;
  let requests = 0;
  globalThis.fetch = (input) => {
    if (String(input) === "/api/plugins") {
      return Promise.resolve(Response.json({
        providers: [
          entry(installed),
          entry(latest, `sha256:${"2".repeat(64)}`),
          entry(unsupported),
        ],
        authentications: [],
        authentication_executors: [],
        platform: { hosts: [] },
      }));
    }
    requests++;
    assertEquals(String(input), "/api/usage/original-account");
    return Promise.resolve(Response.json(usageSnapshot("original-account")));
  };
  try {
    resetProviderCatalog();
    await loadProviderCatalog();
    await refreshSessionUsage(installed.id, installed.version, digest);
    await assertRejects(
      () => refreshSessionUsage(installed.id, "0.0.1", digest),
      Error,
      "Usage is unavailable",
    );
    await assertRejects(
      () => refreshSessionUsage("missing"),
      Error,
      "Usage is unavailable",
    );
    await assertRejects(
      () => refreshSessionUsage(unsupported.id),
      Error,
      "Usage is unavailable",
    );
    assertEquals(requests, 1);
  } finally {
    globalThis.fetch = previousFetch;
    resetProviderCatalog();
  }
});

Deno.test("global usage refresh and cancellable reads retain their HTTP semantics", async () => {
  const previousFetch = globalThis.fetch;
  const controller = new AbortController();
  const requests: Array<[string, string, AbortSignal | null | undefined]> = [];
  globalThis.fetch = (input, init) => {
    requests.push([String(input), init?.method ?? "GET", init?.signal]);
    return Promise.resolve(Response.json(usageSnapshot("openai")));
  };
  try {
    assertEquals(await readUsage(controller.signal), usageSnapshot("openai"));
    assertEquals(await refreshUsage(), usageSnapshot("openai"));
    assertEquals(requests, [["/api/usage", "GET", controller.signal], [
      "/api/usage",
      "POST",
      undefined,
    ]]);
    globalThis.fetch = () =>
      Promise.resolve(new Response("Machine is reconnecting", { status: 503 }));
    await assertRejects(
      () => readUsage(),
      Error,
      "Could not load usage (HTTP 503): Machine is reconnecting",
    );
    await assertRejects(
      () => refreshUsage(),
      Error,
      "Could not refresh usage (HTTP 503): Machine is reconnecting",
    );
  } finally {
    globalThis.fetch = previousFetch;
  }
});
