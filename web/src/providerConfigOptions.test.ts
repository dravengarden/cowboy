import { assertEquals, assertNotStrictEquals } from "jsr:@std/assert";
import type { ConfigOption } from "./protocol";
import {
  providerConfigOptionDisabled,
  providerConfigOptionOrder,
  type ProviderConfigOptionPresentation,
  providerConfigOptions,
  providerConfigSurfaceDisabled,
} from "./providerConfigOptions";

Deno.test("the UI preserves Provider-projected configuration generically", () => {
  const options: ConfigOption[] = [{
    id: "future_option",
    name: "Future option",
    currentValue: "enabled",
    options: [{ value: "enabled", name: "Enabled" }],
  }];
  const projected = providerConfigOptions("future-agent", options);
  assertEquals(projected, options);
  assertNotStrictEquals(projected, options);
});

Deno.test("signed option policy controls layout order and lifecycle without Provider ids", () => {
  const option: ConfigOption = {
    id: "future_option",
    name: "Future option",
    currentValue: "enabled",
    options: [{ value: "enabled", name: "Enabled" }],
  };
  const presentation: ProviderConfigOptionPresentation = {
    id: option.id,
    order: 3,
    layout: "full_width",
    availability: "idle_or_stopped",
  };
  assertEquals(providerConfigOptionOrder(option, presentation), 3);
  assertEquals(providerConfigOptionDisabled("busy", presentation), true);
  assertEquals(
    providerConfigOptionDisabled("interrupted", presentation),
    false,
  );
  assertEquals(
    providerConfigSurfaceDisabled(
      "interrupted",
      [option],
      new Map([[option.id, presentation]]),
    ),
    false,
  );
  assertEquals(providerConfigOptionDisabled("interrupted", undefined), true);
});
