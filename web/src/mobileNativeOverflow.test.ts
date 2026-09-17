import { assert, assertEquals } from "jsr:@std/assert";
import { mobileNativeYScrollSx } from "./mobileNativeOverflow.ts";

const reviewFileTree = await Deno.readTextFile(
  new URL("./mobile/review/ReviewFileTree.tsx", import.meta.url),
);
const reviewChanges = await Deno.readTextFile(
  new URL("./mobile/review/ReviewChanges.tsx", import.meta.url),
);
const codeViewer = await Deno.readTextFile(
  new URL("./mobile/review/CodeViewer.tsx", import.meta.url),
);
const reviewApp = await Deno.readTextFile(
  new URL("./mobile/review/ReviewApp.tsx", import.meta.url),
);
const transcript = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("Agent and Code scrollports keep native vertical momentum", () => {
  assert(mobileNativeYScrollSx.touchAction === "pan-y pinch-zoom");
  assert(reviewFileTree.includes("...mobileNativeYScrollSx"));
  assert(reviewChanges.includes("...mobileNativeYScrollSx"));
  assert(codeViewer.includes("pan-y pinch-zoom"));
  assert(transcript.includes('touchAction: "pan-y pinch-zoom"'));
  // Review document preview (Markdown, Mermaid, media, wrap-on source).
  assert(reviewApp.includes('...(outerScrollable && { touchAction: "pan-y pinch-zoom" })'));
  // Sideways-scrolling Markdown blocks restart touch-action in WebKit.
  assert(reviewApp.includes('touchAction: "pan-x pan-y pinch-zoom"'));
});

Deno.test("Review Markdown wrap is a separate preference in the source wrap slot", () => {
  assert(reviewApp.includes("touchWrap={settings.markdownSoftWrap}"));
  assert(reviewApp.includes("markdownSoftWrap: !settings.markdownSoftWrap"));
  assert(reviewApp.includes('"Wrap Markdown code and tables"'));
  // Reflow keeps the reader on the same block.
  assert(reviewApp.includes("subscribeReviewSettings("));
  assert(reviewApp.includes("settings.markdownSoftWrap, text]"));
  // The symbol sheet is not the document; its hover Markdown still wraps.
  assert(reviewApp.includes("<Markdown text={block.text} touchWrap />"));
});

Deno.test("wrap-on Review source keeps live CodeMirror for workspace swipe", () => {
  assert(codeViewer.includes("bindCodeViewerSwipeFreeze"));
  assert(codeViewer.includes('data-mobile-code-layer="true"'));
  assert(codeViewer.includes('data-mobile-code-wrap'));
  assert(codeViewer.includes("softWrap"));
  assert(codeViewer.includes('position: "relative"'));
  assert(codeViewer.includes("WebkitOverflowScrolling: \"touch\""));
  assert(codeViewer.includes('overflow: softWrap ? "visible" : "auto"'));
  assertEquals(codeViewer.includes("data-mobile-code-snapshot"), false);
  assert(reviewApp.includes("settings.softWrap"));
  assert(reviewApp.includes('data-mobile-overflow-layer'));
});

Deno.test("Code Review samples navigation position after scroll goes idle", () => {
  assert(codeViewer.includes("createMobileCodeScrollIdleReporter"));
  const listenerStart = codeViewer.indexOf("if (onVisibleLine) {");
  const listenerEnd = codeViewer.indexOf(
    "\n    values.push(\n      EditorView.updateListener",
    listenerStart + 1,
  );
  assert(listenerStart >= 0 && listenerEnd > listenerStart);
  const activeScrollListener = codeViewer.slice(listenerStart, listenerEnd);
  assert(activeScrollListener.includes("visibleLineReporterRef.current?.schedule"));
  assertEquals(activeScrollListener.includes("getBoundingClientRect"), false);
  assertEquals(activeScrollListener.includes("documentTop"), false);
});
