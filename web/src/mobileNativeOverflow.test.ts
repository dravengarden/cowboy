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
const transcript = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("Agent and Code scrollports keep native vertical momentum", () => {
  assert(mobileNativeYScrollSx.touchAction === "pan-y pinch-zoom");
  assert(reviewFileTree.includes("...mobileNativeYScrollSx"));
  assert(reviewChanges.includes("...mobileNativeYScrollSx"));
  assert(codeViewer.includes('touchAction: "pan-y pinch-zoom"'));
  assert(transcript.includes('touchAction: "pan-y pinch-zoom"'));
});

Deno.test("Code Review keeps CodeMirror visible on a standing swipe layer", () => {
  assert(codeViewer.includes("bindCodeViewerSwipeFreeze"));
  assert(codeViewer.includes("isMobileCodeSwipeFrozen"));
  assert(codeViewer.includes('data-mobile-code-layer="true"'));
  assertEquals(codeViewer.includes("WebkitOverflowScrolling: \"touch\""), false);
  assertEquals(codeViewer.includes("visibility: \"hidden\""), false);
});
