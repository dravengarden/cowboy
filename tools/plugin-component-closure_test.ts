import { assertEquals, assertThrows } from "jsr:@std/assert@1.0.19";
import {
  type ComponentRelease,
  dependencyClosure,
  validatePluginComponentClosure,
  validateReleaseHistory,
} from "./plugin-component-closure.ts";
import { resolvePackagePins } from "./check-plugin-components.ts";

const digest = (digit: string) => `sha256:${digit.repeat(64)}`;
function history(): ComponentRelease[] {
  const components = ["sdk", "provider", "store", "sync", "idb", "zed"].map((
    id,
  ) => ({
    id: `cowboy.${id}`,
    version: "1.0.0",
    publisher: "cowboy",
    sources: [`components/${id}`],
    digest: digest("0"),
    package: {
      kind: "npm" as const,
      name: `@cowboy/${id}`,
      manifest: `components/${id}/package.json`,
    },
  }));
  const legacy: ComponentRelease = {
    version: "1.0.0",
    components,
    plugins: { codex: "1.0.0", zed: "1.0.0" },
  };
  const baseline: ComponentRelease = {
    ...structuredClone(legacy),
    version: "1.1.0",
    closure: {
      component_dependencies: {
        "cowboy.sdk": ["cowboy.provider"],
        "cowboy.provider": [],
        "cowboy.store": [],
        "cowboy.sync": ["cowboy.store"],
        "cowboy.idb": ["cowboy.sync"],
        "cowboy.zed": [],
      },
      plugins: {
        codex: {
          component_release: "1.0.0",
          components: [{ id: "cowboy.sdk", version: "1.0.0" }],
          source_digest: digest("1"),
        },
        zed: {
          component_release: "1.0.0",
          components: [{ id: "cowboy.zed", version: "1.0.0" }],
          source_digest: digest("2"),
        },
      },
    },
  };
  return [legacy, baseline];
}
function append(releases: ComponentRelease[]): ComponentRelease {
  const next = structuredClone(releases.at(-1)!);
  next.version = "1.2.0";
  releases.push(next);
  return next;
}
function bump(release: ComponentRelease, id: string): void {
  const component = release.components.find((component) =>
    component.id === `cowboy.${id}`
  )!;
  component.version = "2.0.0";
  component.digest = digest("3");
}

Deno.test("schema-2 release history is unchanged by the schema-3 migration", async () => {
  const registry = JSON.parse(
    await Deno.readTextFile("components/registry.json"),
  ) as { releases: ComponentRelease[] };
  const historical = registry.releases.filter((release) =>
    release.closure === undefined
  );
  const hash = new Uint8Array(
    await crypto.subtle.digest(
      "SHA-256",
      new TextEncoder().encode(JSON.stringify(historical)),
    ),
  );
  // Anchored to the complete release history in 6df56fef, not the active entry.
  assertEquals(
    [...hash].map((byte) => byte.toString(16).padStart(2, "0")).join(""),
    "8f44e9e095f32966cb5629a36bd44096bf7a76a1c8e97eeef241e7e9b38e1fb3",
  );
});

Deno.test("closure migration requires a separate unchanged baseline and preserves legacy policy", () => {
  const releases = history();
  validateReleaseHistory(releases);
  bump(releases[1]!, "store");
  assertThrows(
    () => validateReleaseHistory(releases),
    Error,
    "baseline must not change components",
  );
  const backward = history();
  delete append(backward).closure;
  assertThrows(
    () => validateReleaseHistory(backward),
    Error,
    "cannot return to coordinated policy",
  );
});

Deno.test("unrelated Web component changes require their package closure, not Plugin churn", () => {
  const releases = history();
  const next = append(releases);
  for (const id of ["store", "sync", "idb"]) bump(next, id);
  validateReleaseHistory(releases);
  assertEquals(next.plugins, releases[0]!.plugins);
  assertEquals(next.closure!.plugins, releases[1]!.closure!.plugins);
});

Deno.test("transitive package consumers must version even without direct source edits", () => {
  const releases = history();
  bump(append(releases), "store");
  assertThrows(
    () => validateReleaseHistory(releases),
    Error,
    "cowboy.sync: changed component must increase version",
  );
  bump(releases[2]!, "sync");
  assertThrows(
    () => validateReleaseHistory(releases),
    Error,
    "cowboy.idb: changed component must increase version",
  );
});

Deno.test("direct and transitive Plugin inputs cannot be hidden behind old release labels", () => {
  for (const id of ["sdk", "provider"]) {
    const releases = history();
    const next = append(releases);
    bump(next, id);
    if (id === "provider") bump(next, "sdk");
    assertThrows(
      () => validateReleaseHistory(releases),
      Error,
      "changed component closure",
    );
    next.closure!.plugins.codex!.component_release = next.version;
    next.closure!.plugins.codex!.components[0]!.version = "2.0.0";
    assertThrows(
      () => validateReleaseHistory(releases),
      Error,
      "changed Plugin source or binding must increase version",
    );
    next.plugins.codex = "2.0.0";
    next.closure!.plugins.codex!.source_digest = digest("4");
    validateReleaseHistory(releases);
    assertEquals(next.plugins.zed, "1.0.0");
  }
});

Deno.test("Plugin source-only changes and relabels require new identities", () => {
  for (const mutation of ["source", "binding"] as const) {
    const releases = history();
    const next = append(releases);
    const snapshot = next.closure!.plugins.codex!;
    if (mutation === "source") snapshot.source_digest = digest("5");
    else snapshot.component_release = next.version;
    assertThrows(
      () => validateReleaseHistory(releases),
      Error,
      "changed Plugin source or binding must increase version",
    );
    next.plugins.codex = "1.0.1";
    validateReleaseHistory(releases);
  }
});

Deno.test("a codec/package source change cannot retain the same component version", () => {
  const releases = history();
  append(releases).components[2]!.digest = digest("6");
  assertThrows(
    () => validateReleaseHistory(releases),
    Error,
    "changed component must increase version",
  );
});

Deno.test("closure rejects missing, duplicated, unknown, and cyclic nodes", () => {
  assertEquals(dependencyClosure(["a"], { a: ["b"], b: ["c"], c: [] }), [
    "a",
    "b",
    "c",
  ]);
  assertThrows(
    () => dependencyClosure(["a"], { a: ["b"] }),
    Error,
    "missing component dependency node",
  );
  assertThrows(
    () => dependencyClosure(["a"], { a: ["b", "b"], b: [] }),
    Error,
    "duplicate",
  );
  assertThrows(
    () => dependencyClosure(["a"], { a: ["b"], b: ["a"] }),
    Error,
    "cycle",
  );
  const releases = history();
  delete releases[1]!.closure!.component_dependencies["cowboy.store"];
  assertThrows(
    () => validateReleaseHistory(releases),
    Error,
    "identity set mismatch",
  );
});

Deno.test("binding rejects dangling release, duplicate pins, stale pins and missing components", () => {
  const releases = history();
  const original = releases[1]!.closure!.plugins.codex!;
  assertThrows(
    () =>
      validatePluginComponentClosure(releases, "codex", {
        ...original,
        component_release: "99.0.0",
      }),
    Error,
    "unknown component release",
  );
  assertThrows(
    () =>
      validatePluginComponentClosure(releases, "codex", {
        ...original,
        components: [...original.components, ...original.components],
      }),
    Error,
    "duplicate",
  );
  assertThrows(
    () =>
      validatePluginComponentClosure(releases, "codex", {
        ...original,
        components: [{ id: "cowboy.sdk", version: "0.0.1" }],
      }),
    Error,
    "pin differs",
  );
  assertThrows(
    () =>
      validatePluginComponentClosure(releases, "codex", {
        ...original,
        components: [{ id: "cowboy.missing", version: "1.0.0" }],
      }),
    Error,
    "pin differs",
  );
});

Deno.test("package metadata cannot mask stale, range or unregistered internal pins", () => {
  const components = history()[0]!.components;
  assertEquals(
    resolvePackagePins("consumer", {
      "@cowboy/store": "1.0.0",
      react: "^19.0.0",
    }, components),
    ["cowboy.store"],
  );
  for (const version of ["^1.0.0", "0.9.0", "file:../store", "workspace:*"]) {
    assertThrows(
      () =>
        resolvePackagePins(
          "consumer",
          { "@cowboy/store": version },
          components,
        ),
      Error,
      "stale or non-exact",
    );
  }
  assertThrows(
    () =>
      resolvePackagePins("consumer", { "@cowboy/hidden": "1.0.0" }, components),
    Error,
    "unregistered",
  );
});

Deno.test("closure snapshots cannot silently remove a Plugin or regress versions", () => {
  const releases = history();
  const next = append(releases);
  delete next.closure!.plugins.zed;
  assertThrows(
    () => validateReleaseHistory(releases),
    Error,
    "Plugin closure nodes",
  );
  next.closure!.plugins.zed = releases[1]!.closure!.plugins.zed!;
  next.plugins.codex = "0.9.0";
  assertThrows(
    () => validateReleaseHistory(releases),
    Error,
    "Plugin version regressed",
  );
});
