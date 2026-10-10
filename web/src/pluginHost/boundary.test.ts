import { readdir, readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

test("the entire production Web tree has no legacy SDK runtime dependency", async () => {
  async function visit(directory: URL): Promise<void> {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = new URL(
        entry.name + (entry.isDirectory() ? "/" : ""),
        directory,
      );
      if (entry.isDirectory()) await visit(path);
      else if (/\.tsx?$/.test(entry.name) && !entry.name.endsWith(".test.ts")) {
        const source = await readFile(path, "utf8");
        for (
          const forbidden of [
            "@cowboy/plugin-api",
            "components/plugin-api",
            "__COWBOY_NATIVE_PLUGIN_HOST",
            "installPluginRenderers",
            "invokeNativePluginCapability",
          ]
        ) {
          assertEquals(
            source.includes(forbidden),
            false,
            `${path.pathname}: ${forbidden}`,
          );
        }
      }
    }
  }
  await visit(new URL("../", import.meta.url));
  const manifest = JSON.parse(
    await readFile(new URL("../../package.json", import.meta.url), "utf8"),
  );
  assertEquals("@cowboy/plugin-api" in manifest.dependencies, false);
});

test("slot rendering is synchronous, observed and keyed to the exact binding", async () => {
  const source = await readFile(
    new URL("./PluginSlot.tsx", import.meta.url), "utf8",
  );
  assert(source.includes("useSyncExternalStore"));
  assert(source.includes("key={selection.key}"));
  assert(source.includes("class PluginSlotBoundary"));
  assert(source.includes("placeholder"));
  assert(source.includes('display: "contents"'));
  for (
    const forbidden of [
      "fetch(",
      "useEffect",
      ".then(",
      "context?: unknown",
      "as PluginSlotProps",
      "../pluginHost.ts",
      "../ProviderSurface",
      "../pluginUsage",
    ]
  ) {
    assertEquals(source.includes(forbidden), false, forbidden);
  }
  const renderer = await readFile(
    new URL("../pluginHost.ts", import.meta.url), "utf8",
  );
  assertEquals(renderer.includes("context as"), false);
  assertEquals(renderer.includes("contextRecord"), false);
  const main = await readFile(new URL("../main.tsx", import.meta.url), "utf8");
  assert(main.includes("ownPluginHostLifecycle(globalThis)"));
  assert(main.includes("dispose(releasePluginHostScope)"));
});
