import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_REVIEW_SETTINGS,
  normalizeReviewSettings,
} from "./reviewSettingsModel.ts";

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
