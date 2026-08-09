import { assertEquals } from "jsr:@std/assert";
import {
  nextMobileProduct,
  pagerDirectionAllowed,
  pagerOffset,
  pagerTargetOffset,
  predictPagerOffset,
  shouldReservePagerStart,
} from "./appPagerMotion.ts";
import { isDominantVerticalPan } from "../touchGestures.ts";

Deno.test("pager prediction removes one-frame lag without escaping the rail", () => {
  assertEquals(predictPagerOffset(-120, -0.5, 16, 390), -128);
  assertEquals(predictPagerOffset(-380, -2, 24, 390), -390);
  assertEquals(predictPagerOffset(-8, 2, 24, 390), 0);
});

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

Deno.test("interactive content and open spatial drawers keep their gesture", () => {
  assertEquals(shouldReservePagerStart(false), true);
  assertEquals(shouldReservePagerStart(true), false);
  // Agent's left drawer closes leftward and Review's right drawer closes
  // rightward: both directions overlap the product pager, so drawer ownership
  // must disable pager reservation symmetrically.
  assertEquals(shouldReservePagerStart(false, true), false);
  assertEquals(shouldReservePagerStart(true, true), false);
});

Deno.test("product transitions are symmetric", () => {
  assertEquals(nextMobileProduct("agent"), "review");
  assertEquals(nextMobileProduct("review"), "agent");
});

Deno.test("vertical transcript pans release horizontal recognizers", () => {
  assertEquals(isDominantVerticalPan(10, 11), false);
  assertEquals(isDominantVerticalPan(7, 13), true);
  assertEquals(isDominantVerticalPan(30, 8), false);
});
