import { assert, assertEquals } from "jsr:@std/assert";
import {
  bindCodeViewerSwipeFreeze,
  isMobileCodeSwipeFrozen,
  mobileCodeRestLayerSx,
  mobileCodeSwipeFlattenSx,
  MOBILE_CODE_SWIPE_END,
  MOBILE_CODE_SWIPE_START,
  swipeOwnsCodeSurface,
} from "./mobileCodeSurface.ts";

function fakeView() {
  return {
    scrollDOM: {} as HTMLElement,
    dom: {} as HTMLElement,
  };
}

Deno.test("wrap-on rest layer stays visible and does not snapshot", () => {
  assertEquals(mobileCodeRestLayerSx.overflow, "hidden");
  assertEquals(
    Object.prototype.hasOwnProperty.call(mobileCodeRestLayerSx, "visibility"),
    false,
  );
  assertEquals(mobileCodeRestLayerSx.contain, "paint");
  assert(
    Object.prototype.hasOwnProperty.call(
      mobileCodeRestLayerSx,
      "& .cm-scroller, & .cm-content, & .cm-gutters, & .cm-layer",
    ),
  );
});

Deno.test("claimed swipe flattens live code into one filter texture", () => {
  assertEquals(
    mobileCodeSwipeFlattenSx["& [data-mobile-code-layer]"].filter,
    "opacity(0.999)",
  );
  assertEquals(
    Object.prototype.hasOwnProperty.call(
      mobileCodeSwipeFlattenSx["& [data-mobile-code-layer]"],
      "visibility",
    ),
    false,
  );
});

Deno.test("code swipe freeze is reference-counted and skips after release", () => {
  const disposeFirst = bindCodeViewerSwipeFreeze(fakeView(), () => false);
  const disposeSecond = bindCodeViewerSwipeFreeze(fakeView(), () => false);
  assertEquals(isMobileCodeSwipeFrozen(), false);
  globalThis.dispatchEvent(new Event(MOBILE_CODE_SWIPE_START));
  assert(isMobileCodeSwipeFrozen());
  globalThis.dispatchEvent(new Event(MOBILE_CODE_SWIPE_END));
  assertEquals(isMobileCodeSwipeFrozen(), false);
  disposeFirst();
  disposeSecond();
});

Deno.test("a late-mounted editor freezes when a swipe is already claimed", () => {
  assert(
    swipeOwnsCodeSurface((selector) =>
      selector.includes("data-mobile-product-moving") ? {} : null
    ),
  );
  const dispose = bindCodeViewerSwipeFreeze(fakeView(), () => true);
  assert(isMobileCodeSwipeFrozen());
  dispose();
  assertEquals(isMobileCodeSwipeFrozen(), false);
});
