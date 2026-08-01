import { assertEquals } from "jsr:@std/assert";
import {
  mobileDrawerProgress,
  predictDrawerOffset,
  sessionDrawerTargetScroll,
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

Deno.test("session drawer preserves a current row already in the comfort band", () => {
  assertEquals(sessionDrawerTargetScroll({
    currentScroll: 400,
    viewportHeight: 600,
    contentHeight: 2000,
    itemTop: 600,
    itemHeight: 80,
  }), 400);
});

Deno.test("session drawer positions an offscreen current row above centre", () => {
  assertEquals(sessionDrawerTargetScroll({
    currentScroll: 0,
    viewportHeight: 600,
    contentHeight: 2000,
    itemTop: 1000,
    itemHeight: 80,
  }), 824);
});

Deno.test("session drawer clamps current rows at both list edges", () => {
  assertEquals(sessionDrawerTargetScroll({
    currentScroll: 700,
    viewportHeight: 600,
    contentHeight: 2000,
    itemTop: 0,
    itemHeight: 80,
  }), 0);
  assertEquals(sessionDrawerTargetScroll({
    currentScroll: 0,
    viewportHeight: 600,
    contentHeight: 2000,
    itemTop: 1940,
    itemHeight: 60,
  }), 1400);
});
