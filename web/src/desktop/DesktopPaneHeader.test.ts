import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert } from "@std/assert";

const workspaceSource = await readFile(
  new URL("./DesktopWorkspace.tsx", import.meta.url), "utf8",
);

test("Desktop pane actions scroll horizontally instead of shrinking controls", () => {
  assert(
    /lineHeight: 1,\s+flexShrink: 0,/u.test(workspaceSource),
  );
  assert(workspaceSource.includes("data-desktop-pane-action-rail"));
  assert(workspaceSource.includes('overflowX: "auto"'));
  assert(workspaceSource.includes('overscrollBehaviorX: "contain"'));
  assert(workspaceSource.includes('WebkitOverflowScrolling: "touch"'));
  assert(workspaceSource.includes('"&::-webkit-scrollbar": { height: 4 }'));
  assert(workspaceSource.includes("data-desktop-pane-action-track"));
  assert(workspaceSource.includes('width: "max-content"'));
  assert(workspaceSource.includes('minWidth: "100%"'));
  assert(workspaceSource.includes('"& > *": { flexShrink: 0 }'));
});
