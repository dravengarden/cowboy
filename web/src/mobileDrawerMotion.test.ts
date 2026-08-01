import { assertEquals } from "jsr:@std/assert";
import {
  mobileDrawerProgress,
  predictDrawerOffset,
} from "./mobileDrawerMotion.ts";

Deno.test("mobile drawer prediction removes one-frame finger lag without running away", () => {
  assertEquals(predictDrawerOffset(120, 0.5, 8), 124);
  assertEquals(predictDrawerOffset(120, 4, 16), 130);
  assertEquals(predictDrawerOffset(120, -4, 16), 110);
  assertEquals(predictDrawerOffset(120, 1, -5), 120);
});

Deno.test("mobile drawer progress follows the finger", () => {
  assertEquals(mobileDrawerProgress(0, 360), 0);
  assertEquals(mobileDrawerProgress(180, 360), 0.5);
  assertEquals(mobileDrawerProgress(360, 360), 1);
});

Deno.test("mobile drawer progress clamps rubber-band overscroll", () => {
  assertEquals(mobileDrawerProgress(-20, 360), 0);
  assertEquals(mobileDrawerProgress(400, 360), 1);
  assertEquals(mobileDrawerProgress(20, 0), 0);
});
