import { assertEquals } from "jsr:@std/assert";

const themeSource = await Deno.readTextFile(
  new URL("./theme.ts", import.meta.url),
);
const frontendDesign = await Deno.readTextFile(
  new URL("../../docs/architecture/09-frontend.md", import.meta.url),
);
const providerSurfaceSource = await Deno.readTextFile(
  new URL("./ProviderSurface.tsx", import.meta.url),
);
const providerManagementSource = await Deno.readTextFile(
  new URL("./ProviderManagement.tsx", import.meta.url),
);

Deno.test("functional Button icons follow Cowboy's global font scale", () => {
  assertEquals(
    themeSource.includes(
      '"& .MuiButton-startIcon.MuiButton-icon > :nth-of-type(1), & .MuiButton-endIcon.MuiButton-icon > :nth-of-type(1)"',
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

Deno.test("Provider management marks and labels follow Cowboy's global font scale", () => {
  assertEquals(
    providerSurfaceSource.includes(
      "const scaledSize = `calc(${size}px * var(--cowboy-font-scale, 1))`;",
    ),
    true,
  );
  assertEquals(
    providerSurfaceSource.includes("width: scaledSize"),
    true,
  );
  assertEquals(
    providerSurfaceSource.includes("height: scaledSize"),
    true,
  );
  assertEquals(
    providerManagementSource.includes('fontSize: "0.8125rem"'),
    true,
  );
});
