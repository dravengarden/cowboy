import { assertEquals } from "jsr:@std/assert";

const themeSource = await Deno.readTextFile(
  new URL("./theme.ts", import.meta.url),
);
const frontendDesign = await Deno.readTextFile(
  new URL("../../docs/architecture/09-frontend.md", import.meta.url),
);

Deno.test("functional Button icons follow Cowboy's global font scale", () => {
  assertEquals(
    themeSource.includes(
      '"& .MuiButton-startIcon > *, & .MuiButton-endIcon > *"',
    ),
    true,
  );
  assertEquals(themeSource.includes('fontSize: "1.25rem"'), true);
  assertEquals(themeSource.includes('fontSize: "1.125rem"'), true);
  assertEquals(frontendDesign.includes("This is a core visual invariant:"), true);
  assertEquals(
    frontendDesign.includes("Do not introduce a fixed-pixel glyph"),
    true,
  );
});
