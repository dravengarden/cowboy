import { assertEquals } from "jsr:@std/assert";
import { directoryPrefetchTargets } from "./directoryPrefetch.ts";

Deno.test("directory prefetch looks ahead one directory layer with a cap", () => {
  assertEquals(
    directoryPrefetchTargets(
      [
        { path: "src", kind: "directory" },
        { path: "README.md", kind: "file" },
        { path: "tests", kind: "directory" },
        { path: "src", kind: "directory" },
        { path: "vendor", kind: "directory", ignored: true },
        { path: "docs", kind: "directory" },
      ],
      2,
    ),
    ["src", "tests"],
  );
});
