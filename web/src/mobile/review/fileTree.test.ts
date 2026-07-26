import { assertEquals } from "jsr:@std/assert";
import { buildFileTree } from "./fileTree.ts";

Deno.test("buildFileTree groups paths and sorts directories before files", () => {
  assertEquals(
    buildFileTree([
      "README.md",
      "src/z.ts",
      "src/a.ts",
      "docs/guide.md",
    ]),
    [
      {
        name: "docs",
        path: "docs",
        kind: "directory",
        children: [{
          name: "guide.md",
          path: "docs/guide.md",
          kind: "file",
          children: [],
        }],
      },
      {
        name: "src",
        path: "src",
        kind: "directory",
        children: [
          { name: "a.ts", path: "src/a.ts", kind: "file", children: [] },
          { name: "z.ts", path: "src/z.ts", kind: "file", children: [] },
        ],
      },
      {
        name: "README.md",
        path: "README.md",
        kind: "file",
        children: [],
      },
    ],
  );
});
