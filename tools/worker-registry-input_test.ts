import {
  assertEquals,
  assertNotEquals,
  assertThrows,
} from "jsr:@std/assert@1.0.19";
import {
  workerRegistryInput,
  workerRegistryProjection,
} from "./worker-registry-input.ts";

function fixture() {
  const baseline = {
    version: "3.0.0",
    components: [
      { id: "cowboy.app-shell", version: "1.0.0", digest: "shell-old" },
      { id: "cowboy.plugin-sdk", version: "1.0.0", digest: "sdk-old" },
    ],
    plugins: { agent: "1.0.0" },
    closure: {
      component_dependencies: {
        "cowboy.app-shell": [],
        "cowboy.plugin-sdk": [],
      } as Record<string, string[]>,
      plugins: {
        agent: {
          component_release: "3.0.0",
          components: [{ id: "cowboy.plugin-sdk", version: "1.0.0" }],
          source_digest: "agent-old",
        },
      },
    },
    component_additions: ["cowboy.plugin-sdk"],
  };
  const next = structuredClone(baseline);
  next.version = "3.1.0";
  next.components[0]!.version = "1.0.1";
  next.components[0]!.digest = "shell-new";
  const { component_additions: _added, ...tail } = next;
  return {
    schema_version: 3,
    active_release: tail.version,
    releases: [baseline, tail],
  };
}

const encode = (value: unknown) => JSON.stringify(value, null, 2) + "\n";

Deno.test("unconsumed shell-only releases retain exact historical worker registry bytes", async () => {
  const registry = fixture();
  const baseline = {
    ...registry,
    active_release: "3.0.0",
    releases: registry.releases.slice(0, 1),
  };
  assertEquals(workerRegistryProjection(encode(registry)), {
    release: "3.0.0",
    source: encode(baseline),
  });
  assertEquals(
    await workerRegistryInput(encode(registry)),
    await workerRegistryInput(encode(baseline)),
  );
  const next = structuredClone(registry.releases[1]!);
  next.version = "3.2.0";
  next.components[0]!.digest = "shell-newer";
  registry.releases.push(next);
  registry.active_release = next.version;
  assertEquals(
    workerRegistryProjection(encode(registry)).source,
    encode(baseline),
  );
});

Deno.test("worker components, Plugin sources, pins and graph changes retain the latest input", async () => {
  for (
    const change of [
      "sdk",
      "plugin-version",
      "plugin-source",
      "plugin-pin",
      "component-edge",
      "plugin-shell-consumer",
    ]
  ) {
    const registry = fixture();
    const next = registry.releases[1]!;
    switch (change) {
      case "sdk":
        next.components[1]!.digest = "sdk-new";
        break;
      case "plugin-version":
        next.plugins.agent = "1.0.1";
        break;
      case "plugin-source":
        next.closure.plugins.agent.source_digest = "agent-new";
        break;
      case "plugin-pin":
        next.closure.plugins.agent.component_release = "3.1.0";
        break;
      case "component-edge":
        next.closure.component_dependencies["cowboy.plugin-sdk"] = [
          "cowboy.app-shell",
        ];
        break;
      case "plugin-shell-consumer":
        next.closure.plugins.agent.components.push({
          id: "cowboy.app-shell",
          version: "1.0.1",
        });
        break;
    }
    assertEquals(
      workerRegistryProjection(encode(registry)).source,
      encode(registry),
      change,
    );
    const baseline = {
      ...registry,
      active_release: "3.0.0",
      releases: registry.releases.slice(0, 1),
    };
    assertNotEquals(
      await workerRegistryInput(encode(registry)),
      await workerRegistryInput(encode(baseline)),
      change,
    );
  }
});

Deno.test("inactive or empty registries cannot produce a worker input", () => {
  const registry = fixture();
  registry.active_release = "unknown";
  assertThrows(() => workerRegistryProjection(encode(registry)));
  registry.releases = [];
  assertThrows(() => workerRegistryProjection(encode(registry)));
});

Deno.test("repository derived input matches the actual append-only component registry", async () => {
  const expected = await workerRegistryInput(
    Deno.readTextFileSync("components/registry.json"),
  );
  assertEquals(
    JSON.parse(Deno.readTextFileSync("components/worker-registry-input.json")),
    expected,
  );
});

Deno.test("the build checker refuses stale or manually pinned inputs without writes", async () => {
  const root = Deno.makeTempDirSync();
  const script =
    new URL("./worker-registry-input.ts", import.meta.url).pathname;
  try {
    Deno.mkdirSync(`${root}/components`);
    const registry = fixture();
    const source = encode(registry);
    Deno.writeTextFileSync(`${root}/components/registry.json`, source);
    const expected = await workerRegistryInput(source);
    const path = `${root}/components/worker-registry-input.json`;
    const check = () =>
      new Deno.Command(Deno.execPath(), {
        args: ["run", "--no-config", "--allow-read", script],
        cwd: root,
        clearEnv: true,
        stdout: "piped",
        stderr: "piped",
      }).output();
    Deno.writeTextFileSync(path, encode(expected));
    assertEquals((await check()).success, true);
    for (
      const invalid of [
        { ...expected, registry_sha256: "0".repeat(64) },
        { ...expected, component_release: "manually-pinned" },
        { ...expected, extra: "undeclared" },
      ]
    ) {
      const retained = encode(invalid);
      Deno.writeTextFileSync(path, retained);
      const result = await check();
      assertEquals(result.success, false);
      assertEquals(
        new TextDecoder().decode(result.stderr).includes(
          "worker registry input is stale",
        ),
        true,
      );
      assertEquals(Deno.readTextFileSync(path), retained);
      assertEquals(
        Deno.readTextFileSync(`${root}/components/registry.json`),
        source,
      );
    }
    Deno.writeTextFileSync(path, encode(expected));
    registry.releases[1]!.components[1]!.digest = "changed-sdk";
    Deno.writeTextFileSync(
      `${root}/components/registry.json`,
      encode(registry),
    );
    assertEquals((await check()).success, false);
    assertEquals(Deno.readTextFileSync(path), encode(expected));
  } finally {
    Deno.removeSync(root, { recursive: true });
  }
});
