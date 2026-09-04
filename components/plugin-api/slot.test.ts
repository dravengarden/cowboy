import { assert, assertEquals } from "jsr:@std/assert";
import { resolvePluginSlotComponent } from "./resolve.ts";

const types = await Deno.readTextFile(new URL("./types.ts", import.meta.url));
const slotSource = await Deno.readTextFile(new URL("./slot.tsx", import.meta.url));

Deno.test("plugin host API version is exact", () => {
  assert(types.includes('PLUGIN_HOST_API_VERSION = "1.0.0"'));
});

Deno.test("closed slot ids include login and provider surfaces", () => {
  assert(types.includes('"login.method"'));
  assert(types.includes('"provider.usage"'));
  assert(types.includes('"account.panel"'));
  assertEquals(types.includes("window.alert"), false);
});

Deno.test("usage-only modules do not steal other provider slots", () => {
  const usage = () => null as unknown as never;
  const module = {
    default: usage,
    slots: { "provider.usage": usage },
  };
  assertEquals(
    resolvePluginSlotComponent(module, "provider.usage"),
    usage,
  );
  assertEquals(resolvePluginSlotComponent(module, "provider.setup"), null);
  assertEquals(resolvePluginSlotComponent(module, "provider.settings"), null);
  assertEquals(
    resolvePluginSlotComponent({ default: usage }, "provider.setup"),
    usage,
  );
});

Deno.test("plugin slot isolates render crashes from the shell", () => {
  assert(slotSource.includes("class PluginSlotBoundary"));
  assert(slotSource.includes("if (this.state.failed) return this.props.fallback"));
  assert(slotSource.includes("loadPluginSlot(pluginId, slot)"));
  assert(slotSource.includes('display: "contents"'));
  assert(slotSource.includes("context={context}"));
  assert(slotSource.includes("placeholder === undefined ? core : placeholder"));
});
