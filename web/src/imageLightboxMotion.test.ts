import { assert, assertAlmostEquals, assertEquals } from "jsr:@std/assert";
import {
  constrainPanAxis,
  FLICK_MIN_VELOCITY,
  NO_DESTINATION_RESISTANCE,
  PAN_EDGE_RESISTANCE,
  projectFlick,
  SWIPE_COMMIT_VELOCITY,
  trackPanVelocity,
} from "../../components/app-shell/image-lightbox-motion.ts";

Deno.test("an axis with no overflow is rigid, not elastic", () => {
  // A wide, short diagram cannot move vertically. Letting it drift at the
  // elastic rate made every horizontal pan wander off-axis and spring back,
  // which is exactly the surface losing the finger.
  assertEquals(constrainPanAxis(120, 0, true), 0);
  assertEquals(constrainPanAxis(-120, 0, true), 0);
  assertEquals(constrainPanAxis(120, 0, false), 0);
});

Deno.test("a held overshoot resists, a settled one clamps", () => {
  assertEquals(constrainPanAxis(40, 100, true), 40);
  assertEquals(
    constrainPanAxis(150, 100, true),
    100 + 50 * PAN_EDGE_RESISTANCE,
  );
  assertEquals(
    constrainPanAxis(-150, 100, true),
    -100 - 50 * PAN_EDGE_RESISTANCE,
  );
  assertEquals(constrainPanAxis(150, 100, false), 100);
  assertEquals(constrainPanAxis(-150, 100, false), -100);
});

Deno.test("velocity smooths across samples instead of trusting one", () => {
  // One jittery sample must not decide a throw; a sustained drag converges on
  // the speed the finger is actually holding.
  assertAlmostEquals(trackPanVelocity(0, 16, 16), 0.35);
  let velocity = 0;
  for (let sample = 0; sample < 12; sample += 1) {
    velocity = trackPanVelocity(velocity, 16, 16);
  }
  assertAlmostEquals(velocity, 1, 0.01);
  // A zero-length sample cannot divide by zero into an infinite throw.
  assert(Number.isFinite(trackPanVelocity(0, 20, 0)));
});

Deno.test("a lift without a throw carries no coast", () => {
  const still = projectFlick(
    { x: 0, y: 0 },
    { x: FLICK_MIN_VELOCITY / 2, y: 0 },
    { x: 500, y: 500 },
  );
  assertEquals(still, null);
  // Fast, but already pinned against the bound it is thrown towards.
  const pinned = projectFlick({ x: 500, y: 0 }, { x: 2, y: 0 }, { x: 500, y: 0 });
  assertEquals(pinned, null);
});

Deno.test("a flick coasts forward and stops at the bound", () => {
  const free = projectFlick({ x: 0, y: 0 }, { x: -1.5, y: 0 }, { x: 4000, y: 0 });
  assert(free !== null);
  assert(free.x < -300, `expected a real coast, got ${free.x}`);
  assertEquals(free.y, 0);

  const clipped = projectFlick({ x: 0, y: 0 }, { x: -1.5, y: 0 }, { x: 80, y: 0 });
  assert(clipped !== null);
  assertEquals(clipped.x, -80);
  // Clipping the distance also shortens the coast — a flick into an edge must
  // not spend a long settle crawling the last few pixels.
  assert(clipped.durationMs < free.durationMs);
});

Deno.test("the coast leaves at the speed the finger did", () => {
  // cubic-bezier(0.32, 0.72, 0, 1) starts at 2.25x its mean speed, so matching
  // distance / (speed * 0.45) to the duration hands the compositor a curve that
  // begins exactly where the drag stopped. Without that continuity the figure
  // visibly hesitates on release.
  const flick = projectFlick({ x: 0, y: 0 }, { x: 0.5, y: 0 }, { x: 4000, y: 0 });
  assert(flick !== null);
  const meanSpeed = flick.x / flick.durationMs;
  assertAlmostEquals(meanSpeed / 0.5, 0.45, 0.02);
});

Deno.test("an unreachable fit-size swipe resists instead of sliding away", () => {
  assert(NO_DESTINATION_RESISTANCE > 0 && NO_DESTINATION_RESISTANCE < 1);
  assert(SWIPE_COMMIT_VELOCITY > FLICK_MIN_VELOCITY);
});
