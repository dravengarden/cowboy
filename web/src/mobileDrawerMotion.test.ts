import { assertEquals } from "jsr:@std/assert";
import {
  drawerProgressAttribute,
  MOBILE_DRAWER_SETTLE_EASING,
  MOBILE_DRAWER_SPRING_DAMPING,
  MOBILE_DRAWER_SPRING_RESPONSE,
  mobileDrawerCardVisual,
  mobileDrawerProgress,
  mobileDrawerRailOffset,
  mobileDrawerSettleDurationMs,
  predictDrawerOffset,
  sessionDrawerTargetScroll,
  stepDrawerSpring,
} from "./mobileDrawerMotion.ts";

Deno.test("mobile drawer prediction removes one-frame finger lag without running away", () => {
  assertEquals(predictDrawerOffset(120, 0.5, 8), 124);
  assertEquals(predictDrawerOffset(120, 4, 16), 130);
  assertEquals(predictDrawerOffset(120, -4, 16), 110);
  assertEquals(predictDrawerOffset(120, 1, -5), 120);
});

Deno.test("drawer settle uses an iOS deceleration window", () => {
  assertEquals(MOBILE_DRAWER_SETTLE_EASING, "cubic-bezier(0.32, 0.72, 0, 1)");
  assertEquals(mobileDrawerSettleDurationMs(1, 0), 360);
  assertEquals(mobileDrawerSettleDurationMs(0, 2), 260);
  assertEquals(mobileDrawerSettleDurationMs(0.5, 0), 330);
});

Deno.test("drawer rail stays still like an Obsidian sidebar", () => {
  assertEquals(mobileDrawerRailOffset(0, 328), 0);
  assertEquals(mobileDrawerRailOffset(164, 328), 0);
  assertEquals(mobileDrawerRailOffset(328, 328), 0);
});

Deno.test("drawer card recedes like an Obsidian workspace", () => {
  assertEquals(mobileDrawerCardVisual(0, 360, true).dim, 0);
  assertEquals(mobileDrawerCardVisual(360, 360, true).dim, 0.22);
  assertEquals(mobileDrawerCardVisual(360, 360, false).dim, 0.16);
  assertEquals(mobileDrawerCardVisual(180, 360, true).dim, 0.11);
  assertEquals(mobileDrawerCardVisual(360, 360, true).radiusPx, 20);
  assertEquals(mobileDrawerCardVisual(360, 360, false).radiusPx, 16);
  assertEquals(Object.hasOwn(mobileDrawerCardVisual(360, 360, true), "scale"), false);
});

Deno.test("drawer spring continues velocity toward the target", () => {
  assertEquals(MOBILE_DRAWER_SPRING_RESPONSE, 0.2);
  assertEquals(MOBILE_DRAWER_SPRING_DAMPING, 1);
  const step = stepDrawerSpring(40, 0.8, 360, 16);
  assertEquals(step.settled, false);
  assertEquals(step.position > 40, true);
  const rest = stepDrawerSpring(360, 0, 360, 16);
  assertEquals(rest.settled, true);
  assertEquals(rest.position, 360);
});

Deno.test("drawer spring reaches the page in an iOS snappy window", () => {
  let position = 0;
  let velocity = 0;
  for (let i = 0; i < 8; i++) {
    const stepped = stepDrawerSpring(position, velocity, 360, 16);
    position = stepped.position;
    velocity = stepped.velocity;
  }
  assertEquals(position > 320, true);
  for (let i = 0; i < 16; i++) {
    const stepped = stepDrawerSpring(position, velocity, 360, 16);
    position = stepped.position;
    velocity = stepped.velocity;
    if (stepped.settled) {
      assertEquals(position, 360);
      return;
    }
  }
  assertEquals(position > 355, true);
});

Deno.test("mobile drawer progress follows the finger", () => {
  assertEquals(mobileDrawerProgress(0, 360), 0);
  assertEquals(mobileDrawerProgress(180, 360), 0.5);
  assertEquals(mobileDrawerProgress(360, 360), 1);
});

Deno.test("drawer progress attribute is presence-only ownership", () => {
  assertEquals(drawerProgressAttribute(0), null);
  assertEquals(drawerProgressAttribute(0.02), null);
  assertEquals(drawerProgressAttribute(0.021), "1");
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
