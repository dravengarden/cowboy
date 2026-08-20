import { assertEquals } from "jsr:@std/assert";
import { RELIABLE_TOUCH_TAP_MOVE_SLOP_PX } from "./touchGestures.ts";
import {
  iosRubberBand,
  OBSIDIAN_DRAWER_COMMIT_PROGRESS,
  OBSIDIAN_DRAWER_FLICK_PX_PER_MS,
  OBSIDIAN_DRAWER_RAIL_SLOP_PX,
  OBSIDIAN_DRAWER_SCROLL_SLOP_PX,
  OBSIDIAN_DRAWER_TRACK_PX,
  obsidianDrawerAbandonsToScroll,
  obsidianDrawerClaimsSwipe,
  obsidianDrawerLockPx,
  obsidianDrawerRubberOffset,
  obsidianDrawerShouldOpen,
  obsidianDrawerShouldPrepare,
  obsidianDrawerVelocityPxPerMs,
  pushDrawerVelocitySample,
} from "./obsidianDrawerGesture.ts";

Deno.test("workspace swipe tracks after two pixels of horizontal intent", () => {
  assertEquals(obsidianDrawerLockPx(false), OBSIDIAN_DRAWER_TRACK_PX);
  assertEquals(obsidianDrawerShouldPrepare(1, 0, true), false);
  assertEquals(obsidianDrawerShouldPrepare(2, 0, true), true);
  assertEquals(obsidianDrawerShouldPrepare(2, 3, true), false);
  assertEquals(obsidianDrawerShouldPrepare(2, 0, false), false);
  assertEquals(obsidianDrawerClaimsSwipe(2, 0, OBSIDIAN_DRAWER_TRACK_PX), {
    direction: "right",
    distance: 2,
  });
  assertEquals(obsidianDrawerClaimsSwipe(8, 7, OBSIDIAN_DRAWER_TRACK_PX), {
    direction: "right",
    distance: 8,
  });
  assertEquals(obsidianDrawerClaimsSwipe(8, 8, OBSIDIAN_DRAWER_TRACK_PX), null);
  assertEquals(obsidianDrawerClaimsSwipe(8, 9, OBSIDIAN_DRAWER_TRACK_PX), null);
});

Deno.test("rail slop stays above a session-row tap", () => {
  assertEquals(
    OBSIDIAN_DRAWER_RAIL_SLOP_PX > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX,
    true,
  );
  assertEquals(obsidianDrawerLockPx(true), OBSIDIAN_DRAWER_RAIL_SLOP_PX);
  assertEquals(
    obsidianDrawerClaimsSwipe(
      RELIABLE_TOUCH_TAP_MOVE_SLOP_PX,
      0,
      OBSIDIAN_DRAWER_RAIL_SLOP_PX,
    ),
    null,
  );
  assertEquals(
    obsidianDrawerClaimsSwipe(
      OBSIDIAN_DRAWER_RAIL_SLOP_PX,
      0,
      OBSIDIAN_DRAWER_RAIL_SLOP_PX,
    ),
    { direction: "right", distance: OBSIDIAN_DRAWER_RAIL_SLOP_PX },
  );
});

Deno.test("scrollable content waits past incidental horizontal tremor", () => {
  const lockPx = obsidianDrawerLockPx(false, true);
  assertEquals(lockPx, OBSIDIAN_DRAWER_SCROLL_SLOP_PX);
  assertEquals(
    obsidianDrawerClaimsSwipe(3, 1, lockPx),
    null,
  );
  assertEquals(obsidianDrawerAbandonsToScroll(4, 25), true);
  assertEquals(
    obsidianDrawerClaimsSwipe(lockPx, 1, lockPx),
    { direction: "right", distance: lockPx },
  );
  assertEquals(
    obsidianDrawerLockPx(true, true),
    OBSIDIAN_DRAWER_RAIL_SLOP_PX,
  );
});

Deno.test("the first clear axis wins between scroll and swipe", () => {
  assertEquals(obsidianDrawerAbandonsToScroll(0, 1), false);
  assertEquals(obsidianDrawerAbandonsToScroll(1, 2), true);
  assertEquals(obsidianDrawerAbandonsToScroll(8, 7), false);
  assertEquals(obsidianDrawerAbandonsToScroll(7, 8), true);
});

Deno.test("release is a flick or the nearer rest state", () => {
  assertEquals(OBSIDIAN_DRAWER_COMMIT_PROGRESS, 0.5);
  assertEquals(obsidianDrawerShouldOpen(0.49, 0), false);
  assertEquals(obsidianDrawerShouldOpen(0.5, 0), true);
  assertEquals(
    obsidianDrawerShouldOpen(0.1, OBSIDIAN_DRAWER_FLICK_PX_PER_MS),
    true,
  );
  assertEquals(
    obsidianDrawerShouldOpen(0.9, -OBSIDIAN_DRAWER_FLICK_PX_PER_MS),
    false,
  );
});

Deno.test("velocity uses the last 100ms of the finger", () => {
  const samples = [];
  pushDrawerVelocitySample(samples, 1000, 10);
  pushDrawerVelocitySample(samples, 1040, 20);
  pushDrawerVelocitySample(samples, 1120, 40);
  assertEquals(samples[0].t, 1040);
  assertEquals(obsidianDrawerVelocityPxPerMs(samples, 1), 0.25);
  assertEquals(obsidianDrawerVelocityPxPerMs(samples, -1), -0.25);
});

Deno.test("overscroll uses the iOS rubber band, not a linear scale", () => {
  assertEquals(obsidianDrawerRubberOffset(80, 320), 80);
  assertEquals(obsidianDrawerRubberOffset(0, 320), 0);
  assertEquals(obsidianDrawerRubberOffset(320, 320), 320);
  const pulled = obsidianDrawerRubberOffset(-80, 320);
  assertEquals(pulled < 0, true);
  assertEquals(pulled > -80, true);
  assertEquals(pulled, -iosRubberBand(80, 320));
  const pushed = obsidianDrawerRubberOffset(400, 320);
  assertEquals(pushed > 320, true);
  assertEquals(pushed < 400, true);
});
