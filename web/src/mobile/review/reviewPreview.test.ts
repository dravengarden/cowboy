import { assertEquals } from "jsr:@std/assert";
import {
  isReviewMediaPath,
  reviewMediaUrl,
  reviewPreviewKind,
} from "./reviewPreview.ts";

Deno.test("review preview classifies images, svg, mermaid, and markdown", () => {
  assertEquals(reviewPreviewKind("logos/heimdall-logo-005.png"), "image");
  assertEquals(reviewPreviewKind("photo.JPEG"), "image");
  assertEquals(reviewPreviewKind("icon.SVG"), "svg");
  assertEquals(reviewPreviewKind("flow.mmd"), "mermaid");
  assertEquals(reviewPreviewKind("chart.mermaid"), "mermaid");
  assertEquals(reviewPreviewKind("README.md"), "markdown");
  assertEquals(reviewPreviewKind("src/main.rs"), "text");
});

Deno.test("review media URLs stay session-scoped and path-encoded", () => {
  assertEquals(isReviewMediaPath("shot.webp"), true);
  assertEquals(isReviewMediaPath("notes.md"), false);
  assertEquals(
    reviewMediaUrl("sess-1", "tmp/heimdall logo.png"),
    "/api/code/sessions/sess-1/file-raw?path=tmp%2Fheimdall+logo.png",
  );
});
