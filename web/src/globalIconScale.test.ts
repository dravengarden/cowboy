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

test("icons outside Desktop-only chrome follow the global font size on Mobile too", async () => {
  // `desktopSize` is fixed pixels on Mobile, and a numeric MUI fontSize is
  // pixels everywhere; shared icon glyphs use `glyphSize` or rem instead.
  // Transcript activity marks keep authored pixels by design.
  const { readdir } = await import("node:fs/promises");
  const root = new URL("./", import.meta.url);
  const files = (await readdir(root, { recursive: true })).filter((file) =>
    file.endsWith(".tsx") && !file.startsWith("desktop/") &&
    !/(test|conformance|BrowserChecks|fixture)/i.test(file) &&
    file !== "ProviderTranscript.tsx"
  );
  const offenders: string[] = [];
  for (const file of files) {
    const source = await readFile(new URL(file, root), "utf8");
    const icons = new Set<string>();
    for (const match of source.matchAll(/import\s*\{([^}]*)\}\s*from\s*"@mui\/icons-material"/g)) {
      for (const part of match[1]!.split(",")) {
        const name = part.trim().split(" as ").at(-1)?.trim();
        if (name) icons.add(name);
      }
    }
    if (icons.size === 0) continue;
    const tag = new RegExp(`<(${[...icons].join("|")})\\b`, "g");
    for (const match of source.matchAll(tag)) {
      let index = match.index + match[0].length;
      let depth = 0;
      while (index < source.length && !(source[index] === ">" && depth === 0)) {
        if (source[index] === "{") depth++;
        else if (source[index] === "}") depth--;
        index++;
      }
      const opening = source.slice(match.index, index);
      if (/desktopSize\(|fontSize:\s*\d/.test(opening)) {
        offenders.push(`${file}: ${opening.replace(/\s+/g, " ").slice(0, 80)}`);
      }
    }
    for (const match of source.matchAll(/"& (?:\.MuiSvgIcon-root|svg|\.MuiButton-(?:start|end)Icon[^"]*)":\s*\{[^}]*\}/g)) {
      if (/desktopSize\(|fontSize:\s*\d/.test(match[0])) offenders.push(`${file}: ${match[0].slice(0, 80)}`);
    }
  }
  assertEquals(offenders, []);
});
