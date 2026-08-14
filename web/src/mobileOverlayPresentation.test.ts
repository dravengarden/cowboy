import { assert, assertStringIncludes } from "jsr:@std/assert";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const composerSurfaceSource = await Deno.readTextFile(
  new URL("./mobileComposerSurface.ts", import.meta.url),
);

Deno.test("draft move snackbar consumes the active light or dark theme", () => {
  assertStringIncludes(composerSource, 'color: "text.primary"');
  assert(
    composerSurfaceSource.includes("return theme.palette.background.paper;"),
  );
  assertStringIncludes(composerSource, 'borderColor: "divider"');
  assertStringIncludes(composerSource, 'backgroundImage: "none"');
});
