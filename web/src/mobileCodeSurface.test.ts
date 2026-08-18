import { assert, assertEquals } from "jsr:@std/assert";
import {
  bindCodeViewerSwipeFreeze,
  isMobileCodeSwipeFrozen,
  mobileCodeRestLayerSx,
  mobileCodeSwipeSwapSx,
  MOBILE_CODE_SWIPE_END,
  MOBILE_CODE_SWIPE_START,
} from "./mobileCodeSurface.ts";

function fakeElement(): HTMLElement {
  const attrs = new Map<string, string>();
  return {
    setAttribute(name: string, value: string) {
      attrs.set(name, value);
    },
    removeAttribute(name: string) {
      attrs.delete(name);
    },
    getAttribute(name: string) {
      return attrs.get(name) ?? null;
    },
    hasAttribute(name: string) {
      return attrs.has(name);
    },
    addEventListener() {},
    removeEventListener() {},
  } as unknown as HTMLElement;
}

Deno.test("code rest layer stays visible", () => {
  assertEquals(
    Object.prototype.hasOwnProperty.call(mobileCodeRestLayerSx, "visibility"),
    false,
  );
  assertEquals(mobileCodeRestLayerSx.overflow, "hidden");
});

Deno.test("workspace swipe swaps to a snapshot instead of hiding the pane", () => {
  assertEquals(
    mobileCodeSwipeSwapSx["& [data-mobile-code-layer] [data-mobile-code-snapshot]"]
      .visibility,
    "visible",
  );
  assertEquals(
    mobileCodeSwipeSwapSx["& [data-mobile-code-layer] .cm-editor"].visibility,
    "hidden",
  );
});

Deno.test("finger-down paints the snapshot before the swipe freeze", () => {
  const paints: string[] = [];
  const layer = fakeElement();
  const dispose = bindCodeViewerSwipeFreeze({
    view: { scrollDOM: fakeElement(), dom: fakeElement() },
    layer,
    canvas: {} as HTMLCanvasElement,
    paint: () => {
      paints.push("paint");
    },
  });
  assertEquals(paints, ["paint"]);
  globalThis.dispatchEvent(new Event("touchstart"));
  assertEquals(paints, ["paint"]);
  globalThis.dispatchEvent(new Event(MOBILE_CODE_SWIPE_START));
  assertEquals(layer.getAttribute("data-mobile-code-frozen"), "true");
  assert(isMobileCodeSwipeFrozen());
  globalThis.dispatchEvent(new Event(MOBILE_CODE_SWIPE_END));
  assertEquals(layer.hasAttribute("data-mobile-code-frozen"), false);
  dispose();
});
