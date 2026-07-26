import { assertEquals } from "jsr:@std/assert";
import {
  nextMobileProduct,
  pagerDirectionAllowed,
  pagerOffset,
  pagerTargetOffset,
  shouldReservePagerStart,
} from "./appPagerMotion.ts";

Deno.test("Agent left swipe and Review right swipe are the only app transitions", () => {
  assertEquals(pagerDirectionAllowed("agent", -40), true);
  assertEquals(pagerDirectionAllowed("agent", 40), false);
  assertEquals(pagerDirectionAllowed("review", 40), true);
  assertEquals(pagerDirectionAllowed("review", -40), false);
});

Deno.test("pager motion follows the finger and clamps at both products", () => {
  assertEquals(pagerOffset("agent", -120, 390), -120);
  assertEquals(pagerOffset("agent", -500, 390), -390);
  assertEquals(pagerOffset("review", 120, 390), -270);
  assertEquals(pagerOffset("review", 500, 390), 0);
  assertEquals(pagerTargetOffset("agent", 390), 0);
  assertEquals(pagerTargetOffset("review", 390), -390);
});

Deno.test("interactive and horizontally scrolling content keeps its gesture", () => {
  assertEquals(shouldReservePagerStart(false), true);
  assertEquals(shouldReservePagerStart(true), false);
  assertEquals(shouldReservePagerStart(false, true), false);
});

Deno.test("product transitions are symmetric", () => {
  assertEquals(nextMobileProduct("agent"), "review");
  assertEquals(nextMobileProduct("review"), "agent");
});
