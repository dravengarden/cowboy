import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const themeSource = await readFile(
  new URL("./theme.ts", import.meta.url), "utf8",
);
const frontendDesign = await readFile(
  new URL("../../docs/architecture/09-frontend.md", import.meta.url), "utf8",
);
const providerSurfaceSource = await readFile(
  new URL("./ProviderSurface.tsx", import.meta.url), "utf8",
);
const providerManagementSource = await readFile(
  new URL("./ProviderManagement.tsx", import.meta.url), "utf8",
);

test("functional Button icons follow Cowboy's global font scale", () => {
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

test("Provider management marks and labels follow Cowboy's global font scale", () => {
  assertEquals(
    providerSurfaceSource.includes(
      "? `calc(${size}px * var(--cowboy-font-scale, 1))`",
    ),
    true,
  );
  assertEquals(
    providerSurfaceSource.includes("scaleWithFont={false}"),
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
  assertEquals(
    providerSurfaceSource.includes("data-provider-mark-stack"),
    true,
  );
  assertEquals(
    providerSurfaceSource.includes(
      "? `calc(${size}px * var(--cowboy-font-scale, 1))`",
    ),
    true,
  );
});
