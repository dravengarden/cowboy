import { assertStringIncludes } from "jsr:@std/assert";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("draft move snackbar consumes the active light or dark theme", () => {
  assertStringIncludes(composerSource, 'color: "text.primary"');
  assertStringIncludes(
    composerSource,
    'alpha(theme.palette.background.paper, theme.palette.mode === "dark" ? 0.96 : 0.94)',
  );
  assertStringIncludes(composerSource, 'borderColor: "divider"');
  assertStringIncludes(composerSource, 'backgroundImage: "none"');
});
