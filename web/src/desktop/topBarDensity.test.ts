import { assert, assertEquals } from "jsr:@std/assert";
import {
  ACTION_ICON_WIDTH_PX,
  TOPBAR_DENSITY_HYSTERESIS_PX,
  topBarDensity,
  topBarWidth,
  USAGE_SEGMENT_WIDTH_PX,
  usageCountdown,
  usageRemainingTone,
} from "./topBarDensity.ts";

const WIDTHS = {
  config: 190,
  usage: 3 * USAGE_SEGMENT_WIDTH_PX + 44,
  actions: 290,
  compactActions: 3 * ACTION_ICON_WIDTH_PX,
  trailing: 130,
};

Deno.test("the countdown answers 'when' in the narrowest truthful form", () => {
  const now = Date.UTC(2026, 8, 16, 12, 0, 0);
  const at = (ms: number): number | undefined =>
    usageCountdown((now + ms) / 1000, now) as string | undefined;
  assertEquals(at(0), "now");
  assertEquals(at(-60_000), "now");
  assertEquals(at(12 * 60_000), "12m");
  assertEquals(at(59 * 60_000), "59m");
  assertEquals(at(60 * 60_000), "1h");
  assertEquals(at((4 * 60 + 58) * 60_000), "4h58m");
  assertEquals(at(24 * 3_600_000), "1d");
  assertEquals(at((3 * 24 + 20) * 3_600_000), "3d20h");
  // No reported reset stays absent rather than inventing a zero.
  assertEquals(usageCountdown(undefined, now), undefined);
});

Deno.test("remaining quota buckets so 0% cannot read like 95%", () => {
  assertEquals(usageRemainingTone(0), "critical");
  assertEquals(usageRemainingTone(10), "critical");
  assertEquals(usageRemainingTone(11), "low");
  assertEquals(usageRemainingTone(25), "low");
  assertEquals(usageRemainingTone(26), "normal");
  assertEquals(usageRemainingTone(100), "normal");
});

Deno.test("density collapses the words before anything leaves the viewport", () => {
  const full = topBarWidth(WIDTHS, "full");
  const compact = topBarWidth(WIDTHS, "compact");
  assertEquals(compact < full, true);
  assertEquals(topBarDensity(full, WIDTHS, "full"), "full");
  assertEquals(topBarDensity(full - 1, WIDTHS, "full"), "compact");
  // Re-expanding must clear the threshold by the hysteresis margin, or a
  // splitter parked on the boundary flickers between densities.
  assertEquals(topBarDensity(full, WIDTHS, "compact"), "compact");
  assertEquals(
    topBarDensity(full + TOPBAR_DENSITY_HYSTERESIS_PX, WIDTHS, "compact"),
    "full",
  );
  // An unmeasured strip keeps what it has instead of flashing.
  assertEquals(topBarDensity(0, WIDTHS, "full"), "full");
  assertEquals(topBarDensity(Number.NaN, WIDTHS, "compact"), "compact");
});

Deno.test("the countdown is what buys the narrower quota segment", () => {
  // 156px was set by `resets Sep 20 02:00 PM`; `Weekly · 3d20h` fits 104.
  assertEquals(USAGE_SEGMENT_WIDTH_PX, 104);
  assertEquals(3 * 156 - 3 * USAGE_SEGMENT_WIDTH_PX, 156);
});

const topBar = await Deno.readTextFile(
  new URL("./DesktopTopBarControls.tsx", import.meta.url),
);
const app = await Deno.readTextFile(new URL("../App.tsx", import.meta.url));

Deno.test("the quota strip spends its width on the countdown, not a stamp", () => {
  assert(topBar.includes("usageCountdown(provider.resetsAt, now)"));
  // The 30s tick is threaded in, so the countdown cannot quietly go stale.
  assert(topBar.includes("now={clock}"));
  // The absolute stamp stays as the fallback for a reset this client cannot
  // place on a clock, and the U panel still prints both forms.
  assert(topBar.includes("`resets ${shortResetTime(provider.resetsAt)}`"));
  assert(topBar.includes("USAGE_SEGMENT_WIDTH_PX"));
  assert(topBar.includes("USAGE_BALANCE_SEGMENT_WIDTH_PX"));
  assertEquals(topBar.includes("balance ? 286 : 156"), false);
  // One segmented control: the group paints the surface, the segments rule.
  assert(topBar.includes("USAGE_TONE_COLOR[usageRemainingTone("));
  assert(topBar.includes("first ? {} : { borderLeft: 1"));
});

Deno.test("density is measured against the room, not the strip's wishes", () => {
  // App.tsx owns the scroller; the strip itself is `max-content` and would
  // measure what it wants rather than what it has.
  assert(app.includes("data-desktop-topbar-scroller"));
  assert(
    topBar.includes(
      'closest<HTMLElement>(\n      "[data-desktop-topbar-scroller]",\n    )',
    ),
  );
  assert(topBar.includes("new ResizeObserver(measure)"));
  assert(topBar.includes("observer.disconnect()"));
  // The previous tier feeds back in so the hysteresis can damp a boundary pane.
  assert(topBar.includes("densityRef.current"));
  assert(topBar.includes("data-desktop-topbar-density={density}"));
  // The strip's own minimum follows the chosen tier, or collapsing the words
  // would not actually buy any room.
  assert(
    topBar.includes(
      '(density === "full" ? sessionActionsMinWidth : compactActionsMinWidth)',
    ),
  );
});
