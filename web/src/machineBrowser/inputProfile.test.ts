import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { devicePortrait, pointerEnvironment, touchInput } from "./inputProfile";

test("touch input follows neko's rule and falls back to the surface", () => {
  const phone = { touchPoints: true, finePointer: false, hover: false };
  assertEquals(touchInput(phone, false), true);
  // A touchscreen laptop or an iPad with a trackpad has a fine pointer.
  assertEquals(touchInput({ ...phone, finePointer: true }, true), false);
  assertEquals(touchInput({ ...phone, hover: true }, true), false);
  assertEquals(touchInput({ touchPoints: false, finePointer: false, hover: false }, true), false);
  assertEquals(touchInput(undefined, true), true);
  assertEquals(touchInput(undefined, false), false);
});

test("the pointer environment reads touch points and the two media queries", () => {
  const matches = new Set(["(hover: hover)"]);
  const scope = {
    matchMedia: (query: string) => ({ matches: matches.has(query) }),
    navigator: { maxTouchPoints: 5 },
  };
  assertEquals(pointerEnvironment(scope), { touchPoints: true, finePointer: false, hover: true });
  assertEquals(pointerEnvironment({ navigator: { maxTouchPoints: 5 } }), undefined);
  assertEquals(
    pointerEnvironment({ ...scope, navigator: { maxTouchPoints: 0 }, ontouchstart: null } as never)
      ?.touchPoints,
    true,
  );
});

test("orientation comes from the screen, never from the frame", () => {
  // The window is short because the soft keyboard is up; the screen says portrait.
  assertEquals(
    devicePortrait({
      screen: { orientation: { type: "portrait-primary" } },
      innerWidth: 390,
      innerHeight: 300,
    }),
    true,
  );
  assertEquals(devicePortrait({ screen: { orientation: { type: "landscape-secondary" } } }), false);
  assertEquals(devicePortrait({ orientation: 90 }), false);
  assertEquals(devicePortrait({ orientation: 180 }), true);
  assertEquals(devicePortrait({ innerWidth: 390, innerHeight: 844 }), true);
  assertEquals(devicePortrait({ innerWidth: 844, innerHeight: 390 }), false);
});
