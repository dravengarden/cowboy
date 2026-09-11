import { assert, assertEquals } from "jsr:@std/assert";

Deno.test("the entire production Web tree has no legacy SDK runtime dependency", async () => {
  async function visit(directory: URL): Promise<void> {
    for await (const entry of Deno.readDir(directory)) {
      const path = new URL(
        entry.name + (entry.isDirectory ? "/" : ""),
        directory,
      );
      if (entry.isDirectory) await visit(path);
      else if (/\.tsx?$/.test(entry.name) && !entry.name.endsWith(".test.ts")) {
        const source = await Deno.readTextFile(path);
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
    await Deno.readTextFile(new URL("../../package.json", import.meta.url)),
  );
  assertEquals("@cowboy/plugin-api" in manifest.dependencies, false);
});

Deno.test("slot rendering is synchronous, observed and keyed to the exact binding", async () => {
  const source = await Deno.readTextFile(
    new URL("./PluginSlot.tsx", import.meta.url),
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
  const renderer = await Deno.readTextFile(
    new URL("../pluginHost.ts", import.meta.url),
  );
  assertEquals(renderer.includes("context as"), false);
  assertEquals(renderer.includes("contextRecord"), false);
  const main = await Deno.readTextFile(new URL("../main.tsx", import.meta.url));
  assert(main.includes("ownPluginHostLifecycle(globalThis)"));
  assert(main.includes("dispose(releasePluginHostScope)"));
});
