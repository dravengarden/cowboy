import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertStringIncludes } from "@std/assert";

const composerSource = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);
const composerSurfaceSource = await readFile(
  new URL("./mobileComposerSurface.ts", import.meta.url), "utf8",
);

test("draft move snackbar consumes the active light or dark theme", () => {
  assertStringIncludes(composerSource, 'color: "text.primary"');
  assert(
    composerSurfaceSource.includes("return theme.palette.background.paper;"),
  );
  assertStringIncludes(composerSource, 'borderColor: "divider"');
  assertStringIncludes(composerSource, 'backgroundImage: "none"');
});
