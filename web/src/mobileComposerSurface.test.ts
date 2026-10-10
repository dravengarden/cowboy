import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const composerSurfaceSource = await readFile(
  new URL("./mobileComposerSurface.ts", import.meta.url), "utf8",
);
const accessoryDockSource = await readFile(
  new URL("./MobileComposerAccessoryDock.tsx", import.meta.url), "utf8",
);
const composerSource = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);

test("focused mobile composer chrome shares one themed hairline family", () => {
  assertEquals(
    composerSurfaceSource.includes(
      "export function mobileComposerOutlineColor",
    ),
    true,
  );
  assertEquals(
    composerSurfaceSource.includes(
      "export function mobileComposerHairlineColor",
    ),
    true,
  );
  assertEquals(
    composerSurfaceSource.includes("export function mobileComposerOutlineGlow"),
    true,
  );
  assertEquals(
    composerSurfaceSource.includes("borderColor: mobileComposerOutlineColor"),
    true,
  );
  assertEquals(
    composerSurfaceSource.includes("boxShadow: mobileComposerOutlineGlow"),
    true,
  );
  assertEquals(
    composerSurfaceSource.includes("alpha(theme.palette.primary.main, 0.42)"),
    false,
  );
});

test("accessory dock rails use the themed hairline instead of gray divider", () => {
  assertEquals(accessoryDockSource.includes("palette.divider"), false);
  assertEquals(
    accessoryDockSource.includes("borderColor: mobileComposerHairlineColor"),
    true,
  );
  assertEquals(
    accessoryDockSource.includes("borderColor: mobileComposerOutlineColor"),
    true,
  );
  assertEquals(
    accessoryDockSource.includes("boxShadow: mobileComposerOutlineGlow"),
    true,
  );
  assertEquals(
    accessoryDockSource.includes('boxShadow: "none"'),
    true,
  );
});

test("compact format-row split uses the same inner hairline", () => {
  assertEquals(
    composerSource.includes("borderTopColor: mobileComposerHairlineColor"),
    true,
  );
  assertEquals(
    composerSource.includes(
      "borderTopColor: (t) => alpha(t.palette.divider, 0.42)",
    ),
    false,
  );
  assertEquals(
    composerSource.includes("borderColor: mobileComposerOutlineColor"),
    true,
  );
});

test("themed outline glow stays on the outer card, not inner rails", () => {
  const glowUses = accessoryDockSource.split("mobileComposerOutlineGlow")
    .length - 1;
  assertEquals(glowUses, 2);
  assertEquals(
    accessoryDockSource.includes(
      "borderLeft: overlay ? 0 : 1,\n        borderColor: mobileComposerHairlineColor",
    ),
    true,
  );
  assertEquals(
    composerSurfaceSource.includes(
      "peek compositor does not grow extra shadow",
    ),
    true,
  );
});
