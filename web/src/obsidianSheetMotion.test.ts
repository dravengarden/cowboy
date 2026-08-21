import { assert, assertEquals } from "jsr:@std/assert";
import {
  OBSIDIAN_SHEET_CLOSED_SCALE,
  OBSIDIAN_SHEET_INSET_PX,
  OBSIDIAN_SHEET_MAX_FRACTION,
  OBSIDIAN_SHEET_RADIUS_PX,
  OBSIDIAN_SHEET_SCRIM_MAX,
  OBSIDIAN_SHEET_SETTLE_EASING,
  OBSIDIAN_SHEET_SETTLE_MS,
  obsidianSheetScale,
  obsidianSheetScrimOpacity,
  obsidianSheetScrimPointerEvents,
  obsidianSheetSettleMs,
  obsidianSheetTransform,
} from "./obsidianSheetMotion.ts";

const sheetSource = await Deno.readTextFile(
  new URL("./Sheet.tsx", import.meta.url),
);
const modalSource = await Deno.readTextFile(
  new URL("./ObsidianSheet.tsx", import.meta.url),
);
const drawerMotion = await Deno.readTextFile(
  new URL("./mobileDrawerMotion.ts", import.meta.url),
);

Deno.test("compact sheet settle matches the Obsidian/iOS drawer cubic", () => {
  assertEquals(OBSIDIAN_SHEET_SETTLE_EASING, "cubic-bezier(0.32, 0.72, 0, 1)");
  assert(drawerMotion.includes(`"${OBSIDIAN_SHEET_SETTLE_EASING}"`));
  assertEquals(OBSIDIAN_SHEET_SETTLE_MS, 240);
  assertEquals(OBSIDIAN_SHEET_INSET_PX, 0);
  assertEquals(OBSIDIAN_SHEET_RADIUS_PX, 18);
  assertEquals(OBSIDIAN_SHEET_MAX_FRACTION, 0.88);
  assertEquals(OBSIDIAN_SHEET_CLOSED_SCALE, 0.96);
  assertEquals(OBSIDIAN_SHEET_SCRIM_MAX, 0.48);
  assertEquals(obsidianSheetSettleMs(true), 1);
  assertEquals(obsidianSheetSettleMs(false), 240);
});

Deno.test("scale and scrim interpolate from closed to open", () => {
  assertEquals(obsidianSheetScale(200, 200), OBSIDIAN_SHEET_CLOSED_SCALE);
  assertEquals(obsidianSheetScale(0, 200), 1);
  assertEquals(obsidianSheetScale(100, 200), 0.98);
  assertEquals(obsidianSheetScale(0, 0), 1);
  assertEquals(obsidianSheetScrimOpacity(200, 200), 0);
  assertEquals(obsidianSheetScrimOpacity(0, 200), OBSIDIAN_SHEET_SCRIM_MAX);
  assertEquals(
    obsidianSheetScrimOpacity(100, 200),
    OBSIDIAN_SHEET_SCRIM_MAX / 2,
  );
  assertEquals(
    obsidianSheetTransform(40, 0.98),
    "translate3d(0, 40px, 0) scale(0.98)",
  );
});

Deno.test("closing scrim remains hit-testable until the sheet unmounts", () => {
  assertEquals(obsidianSheetScrimPointerEvents(0.48, false), "auto");
  assertEquals(obsidianSheetScrimPointerEvents(0, false), "none");
  assertEquals(obsidianSheetScrimPointerEvents(0, true), "auto");
  assert(modalSource.includes('data-obsidian-sheet-scrim="true"'));
  assert(modalSource.includes("useBackdropDismiss<HTMLDivElement>(dismiss)"));
  assert(modalSource.includes("onPointerUp={dismissBackdrop.onPointerUp}"));
});

Deno.test("phone sheets use the compact Obsidian card, not a floating footer pad", () => {
  assert(sheetSource.includes("<ObsidianSheet"));
  assert(sheetSource.includes("!props.cover"));
  assert(modalSource.includes("data-obsidian-sheet"));
  assert(modalSource.includes("OBSIDIAN_SHEET_SETTLE_EASING"));
  assertEquals(modalSource.includes("footerOverlay"), false);
  assert(modalSource.includes("MobileSheetDismiss"));
  // Home indicator is inside the card. Lifting the whole sheet by
  // safe-area would float it off the bottom, unlike Obsidian.
  assertEquals(
    modalSource.includes("calc(${inset} + env(safe-area-inset-bottom"),
    false,
  );
  assert(modalSource.includes("SAFE_INSIDE"));
  assert(modalSource.includes("calc(${inset} + var(--kb-inset, 0px))"));
  assert(modalSource.includes('bgcolor: "background.paper"'));
  assert(modalSource.includes('border: "none"'));
  assertEquals(modalSource.includes('border: "1px solid"'), false);
  assert(
    modalSource.includes(
      'borderRadius: `${String(OBSIDIAN_SHEET_RADIUS_PX)}px ${String(OBSIDIAN_SHEET_RADIUS_PX)}px 0 0`',
    ),
  );
  assertEquals(modalSource.includes("backdropFilter"), false);
  assert(modalSource.includes('justifyContent: "space-between"'));
});

Deno.test("compact sheets can dim the status bar and float dismissal over content", () => {
  assert(modalSource.includes('meta[name="theme-color"]'));
  assert(modalSource.includes("OBSIDIAN_SHEET_SCRIM_MAX"));
  assert(modalSource.includes("floatingDismiss"));
  assert(modalSource.includes("calc(76px"));
  assert(modalSource.includes('<MobileSheetDismiss onClose={dismiss} />'));
  assert(sheetSource.includes('floatingDismiss={props.mobileDismiss === "footer"}'));
});
