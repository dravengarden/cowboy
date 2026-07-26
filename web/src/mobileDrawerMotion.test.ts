import { assertAlmostEquals, assertEquals } from "jsr:@std/assert";
import {
  mobileDrawerSurfaceVisual,
  predictDrawerOffset,
} from "./mobileDrawerMotion.ts";

Deno.test("mobile drawer prediction removes one-frame finger lag without running away", () => {
  assertEquals(predictDrawerOffset(120, 0.5, 8), 124);
  assertEquals(predictDrawerOffset(120, 4, 16), 130);
  assertEquals(predictDrawerOffset(120, -4, 16), 110);
  assertEquals(predictDrawerOffset(120, 1, -5), 120);
});

Deno.test("mobile drawer visual follows the finger from an exact closed state", () => {
  assertEquals(mobileDrawerSurfaceVisual(0, 360, true), {
    progress: 0,
    scale: 1,
    opacity: 1,
  });
  assertEquals(mobileDrawerSurfaceVisual(180, 360, true), {
    progress: 0.5,
    scale: 0.98,
    opacity: 0.8582167352537723,
  });
  assertEquals(mobileDrawerSurfaceVisual(360, 360, true), {
    progress: 1,
    scale: 0.96,
    opacity: 0.66,
  });
});

Deno.test("mobile drawer stays opaque through direction lock then fades smoothly", () => {
  assertEquals(mobileDrawerSurfaceVisual(35, 360, true).opacity, 1);
  const justAfterLock = mobileDrawerSurfaceVisual(45, 360, true).opacity;
  const halfway = mobileDrawerSurfaceVisual(180, 360, true).opacity;
  assertAlmostEquals(justAfterLock, 0.9992275377229081);
  assertEquals(justAfterLock > halfway, true);
  assertEquals(halfway > mobileDrawerSurfaceVisual(360, 360, true).opacity, true);
});

Deno.test("mobile drawer visual clamps rubber-band overscroll", () => {
  assertEquals(
    mobileDrawerSurfaceVisual(-20, 360, false),
    mobileDrawerSurfaceVisual(0, 360, false),
  );
  assertEquals(
    mobileDrawerSurfaceVisual(400, 360, false),
    mobileDrawerSurfaceVisual(360, 360, false),
  );
});

Deno.test("reduced motion preserves the scale while retaining drawer separation", () => {
  assertEquals(mobileDrawerSurfaceVisual(360, 360, true, true), {
    progress: 1,
    scale: 1,
    opacity: 0.84,
  });
});
