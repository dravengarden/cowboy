import { assertEquals } from "jsr:@std/assert";
import {
  drawerProgressOwnsPagerGesture,
  shouldKeepDrawerDepth,
  translatedSurfaceOwnsPagerGesture,
} from "./mobileDrawerDepth.ts";

Deno.test("drawer depth stays while the card is still translated", () => {
  assertEquals(shouldKeepDrawerDepth(true, 0), true);
  assertEquals(shouldKeepDrawerDepth(false, 180), true);
  assertEquals(shouldKeepDrawerDepth(false, 4), false);
  assertEquals(shouldKeepDrawerDepth(false, 0), false);
});

Deno.test("a slid session card owns the product pager", () => {
  assertEquals(translatedSurfaceOwnsPagerGesture("none"), false);
  assertEquals(translatedSurfaceOwnsPagerGesture("translate3d(0px, 0, 0)"), false);
  assertEquals(
    translatedSurfaceOwnsPagerGesture("translate3d(280px, 0, 0)"),
    true,
  );
  assertEquals(translatedSurfaceOwnsPagerGesture("translateX(-220px)"), true);
  assertEquals(drawerProgressOwnsPagerGesture("0"), false);
  assertEquals(drawerProgressOwnsPagerGesture("0.84"), true);
});
