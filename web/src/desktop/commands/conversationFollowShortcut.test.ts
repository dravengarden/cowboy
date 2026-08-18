import { assert, assertEquals } from "jsr:@std/assert";
import { matchesShortcut, parseShortcut } from "./shortcut.ts";
import { workspaceCommandKey } from "./workspaceCommandKey.ts";

const providerSource = await Deno.readTextFile(
  new URL("./DesktopCommandProvider.tsx", import.meta.url),
);
const hostSource = await Deno.readTextFile(
  new URL("./DesktopCommandHost.tsx", import.meta.url),
);

Deno.test("bare F is a physical f, so Follow cannot look up a shifted F", () => {
  assertEquals(
    workspaceCommandKey({ code: "KeyF", key: "f", shiftKey: false }),
    "f",
  );
  assertEquals(
    workspaceCommandKey({ code: "KeyF", key: "Process", shiftKey: false }),
    "f",
  );
  assert(providerSource.includes('key.toLowerCase() === "f"'));
  assertEquals(providerSource.includes('F: "toggle-following"'), false);
  assertEquals(providerSource.includes('f: "toggle-following"'), false);
});

Deno.test("Reading product letters p/v/f are case-insensitive and not Shift-gated", () => {
  const reading = providerSource.slice(
    providerSource.indexOf('if (workspace.productMode === "reading")'),
    providerSource.indexOf("The docked question directory"),
  );
  assert(reading.includes("const product = key.toLowerCase()"));
  assert(reading.includes('product === "f"'));
  assertEquals(reading.includes("&& !event.shiftKey"), false);
});

Deno.test("Follow is also a conversation command so F works without the scroller map", () => {
  assert(hostSource.includes('id: "conversation.toggleFollow"'));
  assert(hostSource.includes('shortcut: "F"'));
  assert(
    matchesShortcut(
      parseShortcut("F"),
      { key: "f", code: "KeyF", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false },
      true,
      true,
    ),
  );
});
