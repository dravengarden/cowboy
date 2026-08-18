import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_REVIEW_SETTINGS,
  loadReviewSettings,
  migrateReviewCodeFontSize,
  normalizeReviewSettings,
  persistReviewSettings,
  REVIEW_CODE_FONT_DEFAULT_GENERATION,
} from "./reviewSettingsModel.ts";

Deno.test("review code font defaults to a compact 8px", () => {
  assertEquals(DEFAULT_REVIEW_SETTINGS.codeFontSize, 8);
  assertEquals(normalizeReviewSettings({}).codeFontSize, 8);
  assertEquals(normalizeReviewSettings({ codeFontSize: 14 }).codeFontSize, 14);
  assertEquals(migrateReviewCodeFontSize(14), 8);
  assertEquals(migrateReviewCodeFontSize(12), 12);
  assertEquals(migrateReviewCodeFontSize(8), 8);
  assertEquals(loadReviewSettings({ codeFontSize: 14 }).codeFontSize, 8);
  assertEquals(loadReviewSettings({ codeFontSize: 12 }).codeFontSize, 12);
  assertEquals(
    loadReviewSettings({
      codeFontSize: 14,
      codeFontDefaultGeneration: REVIEW_CODE_FONT_DEFAULT_GENERATION,
    }).codeFontSize,
    14,
  );
  assertEquals(
    JSON.parse(persistReviewSettings(DEFAULT_REVIEW_SETTINGS))
      .codeFontDefaultGeneration,
    REVIEW_CODE_FONT_DEFAULT_GENERATION,
  );
});

Deno.test("review settings retain code-review-specific choices", () => {
  assertEquals(
    normalizeReviewSettings({
      codeFontSize: 16,
      softWrap: true,
      contextLines: -1,
      showWhitespaceChanges: false,
      diagnostics: false,
      inlayHints: true,
      semanticHighlighting: false,
    }),
    {
      codeFontSize: 16,
      softWrap: true,
      contextLines: -1,
      showWhitespaceChanges: false,
      diagnostics: false,
      inlayHints: true,
      semanticHighlighting: false,
    },
  );
});

Deno.test("review settings accept compact code font sizes", () => {
  assertEquals(normalizeReviewSettings({ codeFontSize: 6 }).codeFontSize, 6);
  assertEquals(normalizeReviewSettings({ codeFontSize: 7 }).codeFontSize, 7);
  assertEquals(normalizeReviewSettings({ codeFontSize: 8 }).codeFontSize, 8);
  assertEquals(normalizeReviewSettings({ codeFontSize: 10 }).codeFontSize, 10);
});

Deno.test("review settings reject stale or malformed values", () => {
  assertEquals(
    normalizeReviewSettings({
      // Theme and ordinary app typography belong to Agent settings. Code font
      // size is Review-local but only accepts supported display presets.
      codeFontSize: 99,
      lineHeight: "wide",
      softWrap: "yes",
      contextLines: 5,
      diagnostics: null,
    }),
    DEFAULT_REVIEW_SETTINGS,
  );
});
