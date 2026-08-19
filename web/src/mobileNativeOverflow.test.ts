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
