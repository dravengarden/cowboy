import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "../../../tools/test-assert.ts";

async function readSrc(relative: string): Promise<string> {
  return await readFile(new URL(relative, import.meta.url), "utf8");
}

test("ObsidianSheet reference-counts dimmed status bar and cleans up dismiss timer", async () => {
  const src = await readSrc("../ObsidianSheet.tsx");
  assert(src.includes("let openStatusBarSheets = 0;"));
  assert(src.includes("acquireDimmedStatusBar(surfaceColor)"));
  assert(src.includes("dismissTimerRef"));
  assert(src.includes("clearTimer(dismissTimerRef)"));
});

test("MobileConnectionBanner stays below open sheets/dialogs and provides 0px clearance fallback", async () => {
  const src = await readSrc("./MobileConnectionBanner.tsx");
  assert(src.includes('pt: "calc(var(--cowboy-system-top-clearance, 0px) + 6px)"'));
  assert(src.includes("zIndex: theme.zIndex.appBar + 2"));
  assertEquals(src.includes("theme.zIndex.tooltip + 1"), false);
});

test("MobileSyncPill enforces 44px touch height and drops dead attention reminder state", async () => {
  const src = await readSrc("./MobileSyncPill.tsx");
  assert(src.includes("minHeight: 44"));
  assertEquals(src.includes("ATTENTION_ACK_KEY"), false);
  assertEquals(src.includes("hideReminder"), false);
});

test("MobileReviewPage loading state shares BOTTOM_NAV_PADDING to avoid bottom jump", async () => {
  const src = await readSrc("./review/MobileReviewPage.tsx");
  assert(src.includes("BOTTOM_NAV_PADDING"));
  assertEquals(src.includes('pb: "max(env(safe-area-inset-bottom), 12px)"'), false);
});

test("ReviewTabStrip cleans up long-press timer, handles lost pointer capture, and shows overflow fades", async () => {
  const src = await readSrc("./review/ReviewTabStrip.tsx");
  assert(src.includes("onLostPointerCapture={() => finishPress(false)}"));
  assert(src.includes('data-review-tabs-overflow="left"'));
  assert(src.includes('data-review-tabs-overflow="right"'));
  assert(src.includes("useEffect(() => () => {\n    if (timer.current !== undefined) globalThis.clearTimeout(timer.current);"));
});

test("ReviewSelectionBar resets error on selection change and meets 44px touch target floor", async () => {
  const src = await readSrc("./review/ReviewSelectionBar.tsx");
  assert(src.includes("[selection.from, selection.to]"));
  assert(src.includes("width: 44, height: 44"));
  assert(src.includes("minHeight: 44"));
});

test("ReviewDocumentFooter renders visible hunk count on touch instead of hover-only title", async () => {
  const src = await readSrc("./review/ReviewDocumentFooter.tsx");
  assert(src.includes("{hunkIndex + 1}/{hunkCount}"));
  assertEquals(src.includes("title={`Hunk"), false);
});

test("Mobile Review search inputs enforce 16px floor to prevent iOS Safari zoom and preserve keyboard on clear", async () => {
  const fileSearch = await readSrc("./review/ReviewFileSearchBar.tsx");
  assert(fileSearch.includes('fontSize: mobile ? "max(16px, 1rem)" : "0.875rem"'));
  assert(fileSearch.includes("onPointerDown={(event) => event.preventDefault()}"));

  const outline = await readSrc("./review/ReviewOutline.tsx");
  assert(outline.includes('minHeight: "55dvh"'));
  assert(outline.includes('sx: { fontSize: "max(16px, 1rem)" }'));
  assert(outline.includes("onPointerDown={(event) => event.preventDefault()}"));

  const compare = await readSrc("./review/ReviewComparePicker.tsx");
  assert(compare.includes('sx: { fontSize: "max(16px, 1rem)" }'));
  assert(compare.includes("Clear branch search"));
  assert(compare.includes("No matching branches"));

  const rows = await readSrc("./review/ReviewChangeRows.tsx");
  assertEquals(rows.includes("minHeight: 34"), false);
  assertEquals(rows.includes("minHeight: 38"), false);
  assert(rows.includes("minHeight: 44"));
});
