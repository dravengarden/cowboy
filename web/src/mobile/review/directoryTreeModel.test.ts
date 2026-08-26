import {
  assert,
  assertEquals,
  assertNotEquals,
  assertStringIncludes,
} from "jsr:@std/assert";
import {
  belongsToDirectorySubtree,
  directoryListingContains,
  directoryParentPath,
  directoryTreeCacheKey,
  directoryTreeCacheScope,
} from "./directoryTreeModel.ts";

Deno.test("directory cache follows a session workspace retarget", () => {
  const sourceScope = directoryTreeCacheScope(
    "sess-1",
    "/Users/example/project",
  );
  const preparedScope = directoryTreeCacheScope(
    "sess-1",
    "/Users/example/worktrees/sess-1",
  );

  assertNotEquals(sourceScope, preparedScope);
  assertNotEquals(
    directoryTreeCacheKey(sourceScope, "cmd"),
    directoryTreeCacheKey(preparedScope, "cmd"),
  );
  assertEquals(
    directoryTreeCacheKey(preparedScope, "cmd"),
    directoryTreeCacheKey(preparedScope, "cmd"),
  );
});

Deno.test("failed directory reconciliation targets its direct parent", () => {
  assertEquals(directoryParentPath("cmd"), "");
  assertEquals(
    directoryParentPath("service/workflow/nodes"),
    "service/workflow",
  );
  assertEquals(directoryParentPath(""), "");

  assert(
    directoryListingContains(
      [
        {
          name: "nodes",
          path: "service/workflow/nodes",
          kind: "directory",
          ignored: false,
        },
      ],
      "service/workflow/nodes",
    ),
  );
  assertEquals(
    directoryListingContains(
      [
        {
          name: "nodes",
          path: "service/workflow/nodes",
          kind: "file",
          ignored: false,
        },
      ],
      "service/workflow/nodes",
    ),
    false,
  );
});

Deno.test("stale directory cleanup is limited to the failed subtree", () => {
  assert(belongsToDirectorySubtree("cmd", "cmd"));
  assert(belongsToDirectorySubtree("cmd/server/main.go", "cmd"));
  assertEquals(belongsToDirectorySubtree("command", "cmd"), false);
  assertEquals(belongsToDirectorySubtree("service/cmd", "cmd"), false);
});

Deno.test("review tree wires workspace identity and stale-folder recovery", async () => {
  const treeSource = await Deno.readTextFile(
    new URL("./ReviewFileTree.tsx", import.meta.url),
  );
  const appSource = await Deno.readTextFile(
    new URL("./ReviewApp.tsx", import.meta.url),
  );

  assertStringIncludes(
    treeSource,
    "directoryTreeCacheScope(sessionId, cwd)",
  );
  assertStringIncludes(treeSource, "await reconcileFailedDirectory(");
  assertStringIncludes(
    treeSource,
    "fetchCodeTree(sessionId, path, controller.signal, true)",
  );
  assertStringIncludes(
    treeSource,
    "onClick={() => onRetryDirectory(entry.path)}",
  );
  assertStringIncludes(
    appSource,
    "JSON.stringify([workspace.sessionId, workspace.cwd])",
  );
});
