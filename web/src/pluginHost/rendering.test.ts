import { assert, assertEquals } from "jsr:@std/assert";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { PluginSlot } from "./PluginSlot.tsx";
import { webPluginHosts } from "./inventory.ts";
import { pluginHostRelease } from "./identity.ts";

function render(
  overrides: Partial<Parameters<typeof PluginSlot>[0]> = {},
): string {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "React");
  Object.defineProperty(globalThis, "React", {
    configurable: true,
    value: React,
  });
  try {
    return renderToStaticMarkup(React.createElement(PluginSlot, {
      pluginId: "example",
      slot: "provider.usage",
      context: {
        kind: "provider.usage",
        provider: "example",
        title: "Usage",
        showTitle: true,
        emptyMessage: "Empty",
        limits: [],
      },
      render: (renderer) => React.createElement("span", null, renderer),
      children: "core fallback",
      placeholder: "pending observation",
      ...overrides,
    }));
  } finally {
    if (previous) Object.defineProperty(globalThis, "React", previous);
    else Reflect.deleteProperty(globalThis, "React");
  }
}
function host(renderer = "provider-usage-v1") {
  return {
    id: "example",
    generation: "a".repeat(64),
    slots: ["provider.usage"],
    ui: { schema_version: 1, renderers: { "provider.usage": renderer } },
  };
}
function replace(rows: unknown): void {
  webPluginHosts.commitRead(webPluginHosts.beginRead("catalog"), rows);
}

Deno.test("core slot static rendering follows observations and never performs an implicit fetch", () => {
  const previous = globalThis.fetch;
  let fetches = 0;
  globalThis.fetch = () => {
    fetches++;
    throw new Error("Views do not fetch");
  };
  webPluginHosts.reset();
  try {
    assert(render().includes("pending observation"));
    replace([host()]);
    assert(render().includes("provider-usage-v1"));
    replace([host("provider-usage-activity-v1")]);
    assert(render().includes("provider-usage-activity-v1"));
    replace([]);
    assert(render().includes("core fallback"));
    assertEquals(fetches, 0);
  } finally {
    globalThis.fetch = previous;
    webPluginHosts.reset();
  }
});

Deno.test("exact or incomplete selection cannot render the default release", () => {
  webPluginHosts.reset();
  try {
    replace([host()]);
    for (
      const release of [
        pluginHostRelease("1.0.0", undefined),
        pluginHostRelease("1.0.0", `sha256:${"b".repeat(64)}`),
      ]
    ) {
      const html = render({ release });
      assert(html.includes("core fallback"));
      assertEquals(html.includes("provider-usage-v1"), false);
    }
  } finally {
    webPluginHosts.reset();
  }
});

Deno.test("static rendering borrows a snapshot without acquiring a live subscription", () => {
  const subscribe = webPluginHosts.subscribe;
  let registrations = 0;
  webPluginHosts.subscribe = (callback) => {
    registrations++;
    return subscribe(callback);
  };
  try {
    render();
    assertEquals(registrations, 0);
  } finally {
    webPluginHosts.subscribe = subscribe;
    webPluginHosts.reset();
  }
});
