import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_REVIEW_SETTINGS,
  normalizeReviewSettings,
} from "./reviewSettingsModel.ts";

Deno.test("review settings retain code-review-specific choices", () => {
  assertEquals(
    normalizeReviewSettings({
      softWrap: true,
      contextLines: -1,
      showWhitespaceChanges: false,
      diagnostics: false,
      inlayHints: true,
      semanticHighlighting: false,
    }),
    {
      softWrap: true,
      contextLines: -1,
      showWhitespaceChanges: false,
      diagnostics: false,
      inlayHints: true,
      semanticHighlighting: false,
    },
  );
});

Deno.test("review settings reject stale or malformed values", () => {
  assertEquals(
    normalizeReviewSettings({
      // Theme, typography, and line spacing belong to Agent settings. Old
      // Review-local values are intentionally discarded during normalization.
      codeFontSize: 99,
      lineHeight: "wide",
      softWrap: "yes",
      contextLines: 5,
      diagnostics: null,
    }),
    DEFAULT_REVIEW_SETTINGS,
  );
});
