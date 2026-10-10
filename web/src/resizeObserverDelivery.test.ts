import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const composerSource = await readFile(
  new URL("./ComposerTextarea.tsx", import.meta.url), "utf8",
);
const geometrySource = await readFile(
  new URL("./floatingComposerGeometry.ts", import.meta.url), "utf8",
);

test("composer defers ResizeObserver layout work to an animation frame", () => {
  assert(composerSource.includes("new ResizeObserver(() =>"));
  assert(composerSource.includes("globalThis.requestAnimationFrame(() =>"));
  assertEquals(
    composerSource.includes("new ResizeObserver(measureNativeOverflow)"),
    false,
  );
});

test("floating geometry coalesces ResizeObserver delivery outside the callback", () => {
  assert(
    geometrySource.includes("new ResizeObserver(queueMeasurementFrame)"),
  );
  assertEquals(geometrySource.includes("new ResizeObserver(measure)"), false);
});
