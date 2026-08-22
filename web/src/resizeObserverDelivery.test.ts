import { assert, assertEquals } from "jsr:@std/assert";

const composerSource = await Deno.readTextFile(
  new URL("./ComposerTextarea.tsx", import.meta.url),
);
const geometrySource = await Deno.readTextFile(
  new URL("./floatingComposerGeometry.ts", import.meta.url),
);

Deno.test("composer defers ResizeObserver layout work to an animation frame", () => {
  assert(composerSource.includes("new ResizeObserver(() =>"));
  assert(composerSource.includes("globalThis.requestAnimationFrame(() =>"));
  assertEquals(
    composerSource.includes("new ResizeObserver(measureNativeOverflow)"),
    false,
  );
});

Deno.test("floating geometry coalesces ResizeObserver delivery outside the callback", () => {
  assert(
    geometrySource.includes("new ResizeObserver(queueMeasurementFrame)"),
  );
  assertEquals(geometrySource.includes("new ResizeObserver(measure)"), false);
});
