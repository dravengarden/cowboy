import { assert, assertEquals } from "jsr:@std/assert";
import {
  installPluginRenderers,
  installPluginRuntimeHosts,
  loadPluginSlot,
  PLUGIN_RENDERER_IDS,
  type PluginRendererRegistry,
  type PluginSlotComponent,
} from "./types.ts";

const types = await Deno.readTextFile(new URL("./types.ts", import.meta.url));
const slotSource = await Deno.readTextFile(
  new URL("./slot.tsx", import.meta.url),
);

function renderer(name: string): PluginSlotComponent {
  return () => name;
}

const renderers = Object.fromEntries(
  PLUGIN_RENDERER_IDS.map((id) => [id, renderer(id)]),
) as PluginRendererRegistry;

Deno.test("plugin renderer contract is closed and data-only", () => {
  assert(types.includes('PLUGIN_HOST_API_VERSION = "1.0.0"'));
  assert(types.includes('PLUGIN_NATIVE_HOST_API_VERSION = "1.0.0"'));
  assert(types.includes("PLUGIN_RENDERER_SCHEMA_VERSION = 1"));
  assert(types.includes('"login-password-v1"'));
  assert(types.includes('"provider-usage-activity-v1"'));
  assertEquals(types.includes("@vite-ignore"), false);
  assertEquals(types.includes("import("), false);
  assertEquals(types.includes("__COWBOY_PLUGIN_HOST"), false);
  assertEquals(types.includes("React:"), false);
  assertEquals(types.includes("auth?:"), false);
});

Deno.test("signed renderer declarations select only Cowboy-owned components", async () => {
  installPluginRenderers(renderers);
  installPluginRuntimeHosts([
    {
      id: "sample",
      generation: "a".repeat(64),
      slots: ["provider.usage"],
      ui: {
        schema_version: 1,
        renderers: { "provider.usage": "provider-usage-v1" },
      },
      native_capabilities: [],
    },
  ], true);
  const loaded = await loadPluginSlot("sample", "provider.usage");
  assertEquals(
    loaded?.({ pluginId: "sample", slot: "provider.usage" }),
    "provider-usage-v1",
  );
  assertEquals(await loadPluginSlot("sample", "provider.settings"), null);
});

Deno.test("exact host generations coexist without falling through to the default", async () => {
  installPluginRenderers(renderers);
  const oldDigest = `sha256:${"a".repeat(64)}`;
  const currentDigest = `sha256:${"b".repeat(64)}`;
  installPluginRuntimeHosts([
    {
      id: "sample",
      plugin_version: "1.0.0",
      artifact_digest: oldDigest,
      generation: "a".repeat(64),
      default_for_id: false,
      slots: ["provider.usage"],
      ui: {
        schema_version: 1,
        renderers: { "provider.usage": "provider-usage-v1" },
      },
      native_capabilities: [],
    },
    {
      id: "sample",
      plugin_version: "2.0.0",
      artifact_digest: currentDigest,
      generation: "b".repeat(64),
      default_for_id: true,
      slots: ["provider.usage"],
      ui: {
        schema_version: 1,
        renderers: { "provider.usage": "provider-usage-activity-v1" },
      },
      native_capabilities: [],
    },
  ], true);

  const old = await loadPluginSlot(
    "sample",
    "provider.usage",
    "1.0.0",
    oldDigest,
  );
  const current = await loadPluginSlot(
    "sample",
    "provider.usage",
    "2.0.0",
    currentDigest,
  );
  const defaultRenderer = await loadPluginSlot("sample", "provider.usage");
  assertEquals(
    old?.({ pluginId: "sample", slot: "provider.usage" }),
    "provider-usage-v1",
  );
  assertEquals(
    current?.({ pluginId: "sample", slot: "provider.usage" }),
    "provider-usage-activity-v1",
  );
  assertEquals(
    defaultRenderer?.({ pluginId: "sample", slot: "provider.usage" }),
    "provider-usage-activity-v1",
  );
  assertEquals(
    await loadPluginSlot(
      "sample",
      "provider.usage",
      "1.0.0",
      `sha256:${"c".repeat(64)}`,
    ),
    null,
  );
});

Deno.test("plugin slot isolates renderer crashes from the shell", () => {
  assert(slotSource.includes("class PluginSlotBoundary"));
  assert(
    slotSource.includes("if (this.state.failed) return this.props.fallback"),
  );
  assert(slotSource.includes("pluginVersion"));
  assert(slotSource.includes("artifactDigest"));
  assert(slotSource.includes('display: "contents"'));
  assert(slotSource.includes("context={context}"));
  assert(slotSource.includes("placeholder === undefined ? core : placeholder"));
});
