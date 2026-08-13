import { assertEquals } from "jsr:@std/assert";
import {
  CONTROL_CENTER_PANEL_ENTER_MS,
  CONTROL_CENTER_PANEL_EXIT_MS,
  CONTROL_CENTER_PANEL_VIEW_TRANSITION_NAME,
  controlCenterViewTransitionStyles,
  controlCenterPanelMotionSx,
} from "./controlCenterMotion.ts";

Deno.test("control center panel exits quickly and settles in more softly", () => {
  assertEquals(CONTROL_CENTER_PANEL_EXIT_MS, 90);
  assertEquals(CONTROL_CENTER_PANEL_ENTER_MS, 180);
  assertEquals(controlCenterPanelMotionSx(false).opacity, 0);
  assertEquals(controlCenterPanelMotionSx(false).transform, "translateY(4px)");
  assertEquals(controlCenterPanelMotionSx(true).opacity, 1);
  assertEquals(controlCenterPanelMotionSx(true).transform, "translateY(0)");
  assertEquals(
    controlCenterPanelMotionSx(true).viewTransitionName,
    CONTROL_CENTER_PANEL_VIEW_TRANSITION_NAME,
  );
});

Deno.test("control center native transition crossfades both panel snapshots", () => {
  const oldPanel =
    `::view-transition-old(${CONTROL_CENTER_PANEL_VIEW_TRANSITION_NAME})`;
  const newPanel =
    `::view-transition-new(${CONTROL_CENTER_PANEL_VIEW_TRANSITION_NAME})`;
  assertEquals(
    controlCenterViewTransitionStyles[oldPanel].animation.includes("180ms"),
    true,
  );
  assertEquals(
    controlCenterViewTransitionStyles[newPanel].animation.includes("180ms"),
    true,
  );
});

Deno.test("control center panel motion respects reduced-motion preference", () => {
  assertEquals(
    controlCenterPanelMotionSx(true)["@media (prefers-reduced-motion: reduce)"],
    { transition: "none", transform: "none" },
  );
});
