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
    /alpha\(\s*theme\.palette\.background\.paper,\s*theme\.palette\.mode === "dark" \? 0\.96 : 0\.94,?\s*\)/
      .test(composerSurfaceSource),
  );
  assertStringIncludes(composerSource, 'borderColor: "divider"');
  assertStringIncludes(composerSource, 'backgroundImage: "none"');
});
