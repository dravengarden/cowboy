import { assert, assertEquals, assertThrows } from "jsr:@std/assert";
import {
  installPluginRuntimeHosts,
  PLUGIN_SLOT_IDS,
} from "../../../components/plugin-api/types.ts";
import {
  HOST_SLOT_IDS,
  isPluginArtifactDigest,
  isPluginGeneration,
  isPluginIdentifier,
  isPluginVersion,
  pluginHostRelease,
} from "./identity.ts";
import {
  createPluginHostInventory,
  type HostInventorySource,
} from "./inventory.ts";

function host(overrides: Record<string, unknown> = {}) {
  return {
    id: "example",
    generation: "a".repeat(64),
    default_for_id: true,
    slots: ["provider.usage"],
    ui: {
      schema_version: 1,
      renderers: { "provider.usage": "provider-usage-v1" },
    },
    native_capabilities: [],
    ...overrides,
  };
}
function exact(
  version = "1.0.0",
  char = "a",
  overrides: Record<string, unknown> = {},
) {
  return host({
    plugin_version: version,
    artifact_digest: `sha256:${char.repeat(64)}`,
    generation: char.repeat(64),
    ...overrides,
  });
}
function auth(id = "identity-source", renderer = "login-oidc-v1") {
  return host({
    id,
    slots: ["login.method"],
    ui: { schema_version: 1, renderers: { "login.method": renderer } },
  });
}
function replace(
  inventory: ReturnType<typeof createPluginHostInventory>,
  rows: unknown,
  source: HostInventorySource = "catalog",
) {
  const read = inventory.beginRead(source);
  try {
    return inventory.commitRead(read, rows);
  } finally {
    inventory.finishRead(read);
  }
}

Deno.test("core retains the current wire slot vocabulary, not the SDK runtime", () => {
  assertEquals(HOST_SLOT_IDS, PLUGIN_SLOT_IDS);
  const core = createPluginHostInventory();
  installPluginRuntimeHosts([host()], true);
  assertEquals(core.resolve("example", "provider.usage"), { kind: "pending" });
  replace(core, [host()]);
  installPluginRuntimeHosts([], true);
  assertEquals(core.resolve("example", "provider.usage").kind, "ready");
  core.dispose();
});

Deno.test("host identities reject partial pins, trailing newlines and invalid versions", () => {
  for (
    const value of ["example\n", "Example", "../example", "", "x".repeat(65)]
  ) assertEquals(isPluginIdentifier(value), false);
  for (const value of ["1.0.0\n", "01.0.0", "1.x", "1.0.0-beta"]) {
    assertEquals(isPluginVersion(value), false);
  }
  assertEquals(isPluginGeneration("a".repeat(64) + "\n"), false);
  assertEquals(isPluginArtifactDigest(`sha256:${"a".repeat(64)}\n`), false);
  assertEquals(pluginHostRelease("1.0.0", undefined), { kind: "unavailable" });
  assertEquals(pluginHostRelease(undefined, `sha256:${"a".repeat(64)}`), {
    kind: "unavailable",
  });
  assertEquals(pluginHostRelease(undefined, undefined), { kind: "default" });
});

Deno.test("host decoding is closed, precise, and never dispatches a native claim", () => {
  const core = createPluginHostInventory();
  const invalid = [
    host({ generation: "latest" }),
    exact("1.0.0", "a", { generation: "b".repeat(64) }),
    host({ plugin_version: "1.0.0" }),
    host({ artifact_digest: `sha256:${"a".repeat(64)}` }),
    host({ slots: ["provider.usage", "provider.usage"] }),
    host({ slots: ["future"] }),
    host({
      ui: {
        schema_version: 2,
        renderers: { "provider.usage": "provider-usage-v1" },
      },
    }),
    host({
      ui: {
        schema_version: 1,
        renderers: { "provider.usage": "login-oidc-v1" },
      },
    }),
    host({ ui: { schema_version: 1, renderers: {} } }),
    host({ ui: undefined }),
    host({
      ui: {
        schema_version: 1,
        renderers: { "provider.usage": "provider-usage-v1" },
        script: "bad",
      },
    }),
    host({ rpc_argv: ["bad"] }),
    host({ native_capabilities: ["webauthn", "webauthn"] }),
    host({ default_for_id: "true" }),
  ];
  for (const row of invalid) {
    assertEquals(replace(core, [row]), []);
    assertEquals(core.resolve("example", "provider.usage"), {
      kind: "missing",
    });
  }
  const rows = replace(
    core,
    [auth("local", "login-password-v1"), auth()],
    "authentication",
  )!;
  assert(rows.every((row) => !("native_capabilities" in row)));
  assertEquals(core.resolve("local", "login.method").kind, "missing");
  assertEquals(core.resolve("identity-source", "login.method").kind, "ready");
  assertEquals(core.resolve("passkey", "account.panel").kind, "missing");
  core.dispose();
});

Deno.test("exact generations coexist; replacements revoke mounted selections without fallback", () => {
  const core = createPluginHostInventory();
  const old = exact("1.0.0", "a", { default_for_id: false });
  const current = exact("2.0.0", "b", {
    ui: {
      schema_version: 1,
      renderers: { "provider.usage": "provider-usage-activity-v1" },
    },
  });
  const oldRelease = pluginHostRelease(old.plugin_version, old.artifact_digest);
  const observed: string[] = [];
  const unsubscribe = core.subscribe(() =>
    observed.push(core.resolve("example", "provider.usage", oldRelease).kind)
  );
  replace(core, [old, current]);
  const pinned = core.resolve("example", "provider.usage", oldRelease);
  const selected = core.resolve("example", "provider.usage");
  assert(pinned.kind === "ready" && selected.kind === "ready");
  assertEquals(pinned.renderer, "provider-usage-v1");
  assertEquals(selected.renderer, "provider-usage-activity-v1");
  assert(pinned.key !== selected.key);
  replace(core, [current]);
  assertEquals(observed, ["ready", "missing"]);
  assertEquals(
    core.resolve("example", "provider.usage", oldRelease).kind,
    "missing",
  );
  unsubscribe();
  core.dispose();
});

Deno.test("duplicate and ambiguous defaults fail closed in every host projection", () => {
  const core = createPluginHostInventory();
  const old = exact();
  assertEquals(replace(core, [old, old, old]), []);
  const rows = replace(core, [old, exact("2.0.0", "b")])!;
  assertEquals(rows.length, 2);
  assert(rows.every((row) => row.default_for_id === false));
  assertEquals(core.resolve("example", "provider.usage").kind, "missing");
  assertEquals(
    core.resolve(
      "example",
      "provider.usage",
      pluginHostRelease("1.0.0", old.artifact_digest),
    ).kind,
    "ready",
  );
  replace(core, [old]);
  assertEquals(core.resolve("example", "provider.usage").kind, "ready");
  replace(core, [{ ...old, default_for_id: false }]);
  assertEquals(core.resolve("example", "provider.usage").kind, "missing");
  core.dispose();
});

Deno.test("authentication and Catalog own independent complete observations", () => {
  const core = createPluginHostInventory();
  replace(core, [host(), auth()]);
  assertEquals(core.resolve("identity-source", "login.method").kind, "pending");
  replace(core, [auth(), host()], "authentication");
  assertEquals(core.resolve("identity-source", "login.method").kind, "ready");
  replace(core, [], "authentication");
  assertEquals(core.resolve("identity-source", "login.method").kind, "missing");
  assertEquals(core.resolve("example", "provider.usage").kind, "ready");
  replace(core, []);
  replace(core, [host()], "authentication");
  assertEquals(core.resolve("example", "provider.usage").kind, "missing");
  core.dispose();
});

Deno.test("host snapshots are detached and frozen; invalid envelopes retain the last observation", () => {
  const core = createPluginHostInventory();
  const raw = host({ visual: { light: { primary: "#000000" } } });
  const rows = replace(core, [raw])!;
  raw.ui.renderers["provider.usage"] = "bad";
  assertEquals(core.resolve("example", "provider.usage").kind, "ready");
  assert(
    Object.isFrozen(rows) && Object.isFrozen(rows[0]) &&
      Object.isFrozen(rows[0]!.visual),
  );
  assertThrows(() => {
    (rows[0] as Record<string, unknown>).id = "other";
  }, TypeError);
  const snapshot = core.getSnapshot();
  for (
    const invalid of [
      null,
      {},
      Array.from({ length: 1025 }, () => host()),
      Array.from({ length: 100 }, () => host({ label: "x".repeat(64000) })),
    ]
  ) {
    assertThrows(() => replace(core, invalid), TypeError);
    assertEquals(core.getSnapshot(), snapshot);
  }
  let deep: unknown = "end";
  for (let i = 0; i < 35; i++) deep = { next: deep };
  assertThrows(() => replace(core, [host({ usage: deep })]), TypeError);
  assertEquals(core.getSnapshot(), snapshot);
  assertEquals(replace(core, [host({ label: "x".repeat(65536) })]), []);
  core.dispose();
});

Deno.test("read observations are owner-bound, single-use, and newest-request fenced", () => {
  const core = createPluginHostInventory();
  const other = createPluginHostInventory();
  const first = core.beginRead("catalog");
  const second = core.beginRead("catalog");
  assert(first.signal.aborted);
  assertEquals(core.commitRead(first, [host()]), undefined);
  assertEquals(other.commitRead(second, [host()]), undefined);
  assertEquals(core.commitRead({ ...second }, [host()]), undefined);
  assert(core.commitRead(second, [host()]));
  assertEquals(core.commitRead(second, []), undefined);
  const ended = core.beginRead("catalog");
  core.finishRead(ended);
  assert(ended.signal.aborted);
  assertEquals(core.commitRead(ended, []), undefined);
  assertEquals(core.resolve("example", "provider.usage").kind, "ready");
  core.dispose();
  other.dispose();
});

Deno.test("subscriptions have independent ownership and idempotent disposal", () => {
  const core = createPluginHostInventory();
  const other = createPluginHostInventory();
  let calls = 0;
  const listener = () => calls++;
  const first = core.subscribe(listener);
  const second = core.subscribe(listener);
  replace(core, [host()]);
  assertEquals(calls, 2);
  first();
  first();
  replace(other, [host()]);
  assertEquals(calls, 2);
  replace(core, []);
  assertEquals(calls, 3);
  second();
  core.dispose();
  core.dispose();
  assertEquals(calls, 3);
  assertEquals(other.resolve("example", "provider.usage").kind, "ready");
  other.dispose();
});

Deno.test("reset aborts only observations and refuses late response resurrection", () => {
  const core = createPluginHostInventory();
  const catalog = core.beginRead("catalog");
  const authentication = core.beginRead("authentication");
  const operation = new AbortController();
  core.reset();
  assert(catalog.signal.aborted && authentication.signal.aborted);
  assertEquals(operation.signal.aborted, false);
  assertEquals(core.commitRead(catalog, [host()]), undefined);
  assertEquals(core.commitRead(authentication, [auth()]), undefined);
  replace(core, [host()]);
  assertEquals(core.resolve("example", "provider.usage").kind, "ready");
  const pending = core.beginRead("catalog");
  let notifications = 0;
  core.subscribe(() => {
    notifications++;
    assertEquals(core.resolve("example", "provider.usage").kind, "missing");
  });
  core.dispose();
  core.dispose();
  assert(pending.signal.aborted);
  assertEquals(notifications, 1);
  assertThrows(() => core.beginRead("catalog"), Error);
});

Deno.test("reentrant resets cannot publish a stale decoded or notified observation", () => {
  const core = createPluginHostInventory();
  const first = core.beginRead("catalog");
  const malicious = host();
  Object.defineProperty(malicious, "label", {
    enumerable: true,
    get: () => {
      core.reset();
      return "label";
    },
  });
  assertEquals(core.commitRead(first, [malicious]), undefined);
  const stop = core.subscribe(() => core.reset());
  assertEquals(replace(core, [host()]), undefined);
  assertEquals(core.resolve("example", "provider.usage").kind, "pending");
  stop();
  core.dispose();
});

Deno.test("a released or throwing observer cannot obstruct another observer", () => {
  const core = createPluginHostInventory();
  const warn = console.warn;
  let warnings = 0;
  console.warn = () => warnings++;
  let releasedCalls = 0;
  let liveCalls = 0;
  let release = () => {};
  const first = core.subscribe(() => {
    release();
    throw new Error("private diagnostic");
  });
  release = core.subscribe(() => releasedCalls++);
  const third = core.subscribe(() => liveCalls++);
  try {
    replace(core, [host()]);
    assertEquals([warnings, releasedCalls, liveCalls], [1, 0, 1]);
  } finally {
    first();
    release();
    third();
    core.dispose();
    console.warn = warn;
  }
});
