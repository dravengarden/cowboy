import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_REVIEW_SETTINGS,
  normalizeReviewSettings,
} from "./reviewSettingsModel.ts";

Deno.test("review settings retain supported display choices", () => {
  assertEquals(
    normalizeReviewSettings({
      codeFontSize: 18,
      lineHeight: 1.7,
      softWrap: true,
      contextLines: -1,
      showWhitespaceChanges: false,
      diagnostics: false,
      inlayHints: true,
      semanticHighlighting: false,
    }),
    {
      codeFontSize: 18,
      lineHeight: 1.7,
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
      codeFontSize: 99,
      lineHeight: "wide",
      softWrap: "yes",
      contextLines: 5,
      diagnostics: null,
    }),
    DEFAULT_REVIEW_SETTINGS,
  );
});

