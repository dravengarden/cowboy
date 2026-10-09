import { assertEquals } from "jsr:@std/assert";
import { providerUiManifestFixture } from "./providerUiContract.fixture.ts";
import {
  loadProviderCatalog,
  peekProviderCatalog,
  reconcileProviderCatalog,
  resetProviderCatalog,
} from "./providerCatalogRegistry.ts";

function digest(seed: string): string {
  return `sha256:${seed.repeat(64)}`;
}

function entry(version: string, artifact: string) {
  const manifest = providerUiManifestFixture();
  manifest.id = "claude-code";
  manifest.version = version;
  return {
    provider_id: manifest.id,
    provider_version: version,
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

function catalog(entries: ReturnType<typeof entry>[]) {
  return {
    providers: entries,
    authentications: [],
    authentication_executors: [],
    platform: { hosts: [] },
  };
}

function installed(version: string, artifact: string, state = "active") {
  return [{
    plugin_id: "claude-code",
    plugin_version: version,
    plugin_kind: "agent_provider",
    generation_digest: artifact,
    contract_fingerprint: artifact,
    state,
    active_session_leases: 0,
    replica_state: "absent",
    materialization_state: "current",
  }];
}

async function settle(): Promise<void> {
  for (let turn = 0; turn < 10; turn++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

Deno.test("a Provider release installed after the Catalog read refreshes the Catalog once", async () => {
  const previous = globalThis.fetch;
  const old = entry("3.19.5", digest("a"));
  const fresh = entry("3.19.6", digest("b"));
  let served = catalog([old]);
  let reads = 0;
  resetProviderCatalog();
  try {
    globalThis.fetch = () => {
      reads++;
      return Promise.resolve(Response.json(served));
    };
    await loadProviderCatalog();
    assertEquals(reads, 1);

    // Inventory the cached Catalog already carries: no read.
    reconcileProviderCatalog([installed("3.19.5", digest("a"))]);
    // Inactive identities do not drive placement and do not trigger a read.
    reconcileProviderCatalog([installed("3.19.6", digest("b"), "installing")]);
    await settle();
    assertEquals(reads, 1);

    served = catalog([old, fresh]);
    reconcileProviderCatalog([installed("3.19.6", digest("b"))]);
    await settle();
    assertEquals(reads, 2);
    assertEquals(
      peekProviderCatalog()?.providers.map((p) => p.provider_version),
      ["3.19.5", "3.19.6"],
    );

    reconcileProviderCatalog([installed("3.19.6", digest("b"))]);
    await settle();
    assertEquals(reads, 2);
  } finally {
    globalThis.fetch = previous;
    resetProviderCatalog();
  }
});

Deno.test("an identity the Service Catalog still lacks is not re-read on every snapshot", async () => {
  const previous = globalThis.fetch;
  let reads = 0;
  resetProviderCatalog();
  try {
    globalThis.fetch = () => {
      reads++;
      return Promise.resolve(
        Response.json(catalog([entry("3.19.5", digest("a"))])),
      );
    };
    await loadProviderCatalog();
    const orphan = [installed("0.0.1", digest("c"))];
    reconcileProviderCatalog(orphan);
    await settle();
    assertEquals(reads, 2);
    reconcileProviderCatalog(orphan);
    reconcileProviderCatalog(orphan);
    await settle();
    assertEquals(reads, 2);
  } finally {
    globalThis.fetch = previous;
    resetProviderCatalog();
  }
});

Deno.test("a snapshot during an in-flight read is judged against that read", async () => {
  const previous = globalThis.fetch;
  const fresh = entry("3.19.6", digest("b"));
  let reads = 0;
  resetProviderCatalog();
  try {
    globalThis.fetch = () => {
      reads++;
      return Promise.resolve(
        Response.json(catalog([entry("3.19.5", digest("a"))])),
      );
    };
    await loadProviderCatalog();
    const delayed = Promise.withResolvers<Response>();
    globalThis.fetch = () => {
      reads++;
      return delayed.promise;
    };
    const inFlight = loadProviderCatalog(true);
    reconcileProviderCatalog([installed("3.19.6", digest("b"))]);
    globalThis.fetch = () => {
      reads++;
      return Promise.resolve(
        Response.json(catalog([entry("3.19.5", digest("a")), fresh])),
      );
    };
    // The in-flight read predates the install and lacks 3.19.6.
    delayed.resolve(Response.json(catalog([entry("3.19.5", digest("a"))])));
    await inFlight;
    await settle();
    assertEquals(reads, 3);
    assertEquals(
      peekProviderCatalog()?.providers.map((p) => p.provider_version),
      ["3.19.5", "3.19.6"],
    );
  } finally {
    globalThis.fetch = previous;
    resetProviderCatalog();
  }
});

Deno.test("a tab that has not read the Catalog does not read it for inventory", async () => {
  const previous = globalThis.fetch;
  let reads = 0;
  resetProviderCatalog();
  try {
    globalThis.fetch = () => {
      reads++;
      return Promise.resolve(Response.json(catalog([])));
    };
    reconcileProviderCatalog([installed("3.19.6", digest("b"))]);
    await settle();
    assertEquals(reads, 0);
  } finally {
    globalThis.fetch = previous;
    resetProviderCatalog();
  }
});
