import { assert } from "jsr:@std/assert";

const workspaceSource = await Deno.readTextFile(
  new URL("./DesktopWorkspace.tsx", import.meta.url),
);

Deno.test("Desktop pane actions scroll horizontally instead of shrinking controls", () => {
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
