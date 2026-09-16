import { assertEquals } from "jsr:@std/assert";
import {
  createRetargetedTouchClickGuard,
  expandedSelection,
  followDetachedTouchStream,
  horizontalSwipe,
  inputOverlayOwnsDrawerGesture,
  isPairedTouchClick,
  isVerticalScrollContainer,
  MOBILE_DRAWER_DIRECTION_LOCK_PX,
  RELIABLE_TOUCH_TAP_MOVE_SLOP_PX,
  shouldFreezePreviewPointer,
  swipeCommits,
} from "./touchGestures.ts";

Deno.test("paired touch click stays suppressed regardless of Safari delay", () => {
  assertEquals(isPairedTouchClick(true, 1), true);
  assertEquals(isPairedTouchClick(true, 2), true);
  assertEquals(isPairedTouchClick(true, 0, "touch"), true);
});

Deno.test("keyboard and assistive clicks are not swallowed by a touch claim", () => {
  assertEquals(isPairedTouchClick(true, 0), false);
  assertEquals(isPairedTouchClick(false, 1), false);
  assertEquals(isPairedTouchClick(false, 0), false);
});

Deno.test("retargeted touch click guard consumes only the completed gesture click", () => {
  const guard = createRetargetedTouchClickGuard();

  guard.arm();
  assertEquals(guard.consume(0), false);
  assertEquals(guard.consume(1, "touch"), true);
  assertEquals(guard.consume(1, "touch"), false);
});

Deno.test("a fresh pointer gesture releases the retargeted click guard", () => {
  const guard = createRetargetedTouchClickGuard();

  guard.arm();
  guard.reset();
  assertEquals(guard.consume(1, "touch"), false);
});

Deno.test("expanded native text selection owns horizontal handle drags", () => {
  assertEquals(expandedSelection(null), false);
  assertEquals(expandedSelection({ rangeCount: 1, isCollapsed: true }), false);
  assertEquals(expandedSelection({ rangeCount: 1, isCollapsed: false }), true);
});

Deno.test("only the focused keyboard overlay reserves the drawer gesture", () => {
  assertEquals(inputOverlayOwnsDrawerGesture(false, false), false);
  assertEquals(inputOverlayOwnsDrawerGesture(false, true), false);
  assertEquals(inputOverlayOwnsDrawerGesture(true, false), false);
  assertEquals(inputOverlayOwnsDrawerGesture(true, true), true);
});

Deno.test("horizontal swipe waits for a deliberate direction lock", () => {
  assertEquals(horizontalSwipe(11, 0), null);
  assertEquals(horizontalSwipe(40, 32), null);
  assertEquals(horizontalSwipe(48, 10), { direction: "right", distance: 48 });
  assertEquals(horizontalSwipe(-48, 10), { direction: "left", distance: 48 });
});

Deno.test("a row tap stops qualifying before the drawer can lock", () => {
  assertEquals(
    MOBILE_DRAWER_DIRECTION_LOCK_PX > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX,
    true,
  );
  assertEquals(
    horizontalSwipe(RELIABLE_TOUCH_TAP_MOVE_SLOP_PX, 0),
    null,
  );
  assertEquals(
    horizontalSwipe(MOBILE_DRAWER_DIRECTION_LOCK_PX, 0),
    { direction: "right", distance: MOBILE_DRAWER_DIRECTION_LOCK_PX },
  );
});

Deno.test("horizontal navigation requires a substantial phone swipe", () => {
  assertEquals(swipeCommits(87, 390), false);
  assertEquals(swipeCommits(94, 390), true);
  assertEquals(swipeCommits(111, 1024), false);
  assertEquals(swipeCommits(112, 1024), true);
});

Deno.test("real vertical overflow remains native instead of reserving a swipe", () => {
  assertEquals(
    isVerticalScrollContainer({ clientHeight: 600, scrollHeight: 1800 }, "auto"),
    true,
  );
  assertEquals(
    isVerticalScrollContainer({ clientHeight: 600, scrollHeight: 600 }, "auto"),
    false,
  );
  assertEquals(
    isVerticalScrollContainer({ clientHeight: 600, scrollHeight: 1800 }, "hidden"),
    false,
  );
});

Deno.test("preview movement freeze belongs only to a primary mouse press", () => {
  assertEquals(shouldFreezePreviewPointer("mouse", 0), true);
  assertEquals(shouldFreezePreviewPointer("mouse", 1), false);
  assertEquals(shouldFreezePreviewPointer("touch", 0), false);
  assertEquals(shouldFreezePreviewPointer("pen", 0), false);
});

Deno.test("a swipe keeps its touch stream after a render detaches the start node", () => {
  class StartNode extends EventTarget {
    isConnected = true;
  }
  const node = new StartNode();
  const seen: string[] = [];
  // Registered first, like the capture-phase pager on the detached path.
  node.addEventListener("touchmove", (event) => {
    if ((event as Event & { claimed?: boolean }).claimed) event.stopPropagation();
  });
  const stop = followDetachedTouchStream(node, {
    move: () => seen.push("move"),
    end: () => seen.push("end"),
    cancel: () => seen.push("cancel"),
  });
  const touch = (type: string, claimed = false) =>
    Object.assign(new Event(type, { cancelable: true }), { claimed });

  // Connected: the gesture root hears the bubbled event; do not double-handle.
  node.dispatchEvent(touch("touchmove"));
  assertEquals(seen, []);

  node.isConnected = false;
  node.dispatchEvent(touch("touchmove"));
  // A recognizer that already claimed and stopped the event keeps it hidden.
  node.dispatchEvent(touch("touchmove", true));
  node.dispatchEvent(touch("touchend"));
  assertEquals(seen, ["move", "end"]);

  stop();
  node.dispatchEvent(touch("touchcancel"));
  assertEquals(seen, ["move", "end"]);
  const noop = { move: () => undefined, end: () => undefined, cancel: () => undefined };
  assertEquals(followDetachedTouchStream(null, noop)(), undefined);
});
