import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { matchesShortcut, parseShortcut } from "./shortcut.ts";
import { workspaceCommandKey } from "./workspaceCommandKey.ts";

const providerSource = await readFile(
  new URL("./DesktopCommandProvider.tsx", import.meta.url), "utf8",
);
const hostSource = await readFile(
  new URL("./DesktopCommandHost.tsx", import.meta.url), "utf8",
);

test("bare F is a physical f, so Follow cannot look up a shifted F", () => {
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

test("Follow is also a conversation command so F works without the scroller map", () => {
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
