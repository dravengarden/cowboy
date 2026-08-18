import { assert, assertEquals } from "jsr:@std/assert";
import {
  bindCodeViewerSwipeFreeze,
  hideMobileCodePaint,
  isMobileCodeSwipeFrozen,
  MOBILE_CODE_SWIPE_END,
  MOBILE_CODE_SWIPE_START,
  showMobileCodePaint,
  swipeOwnsCodeSurface,
} from "./mobileCodeSurface.ts";

function fakeElement(): HTMLElement {
  const attrs = new Map<string, string>();
  const props = new Map<string, string>();
  const style = {
    set visibility(value: string) {
      props.set("visibility", value);
    },
    get visibility() {
      return props.get("visibility") ?? "";
    },
    set pointerEvents(value: string) {
      props.set("pointer-events", value);
    },
    get pointerEvents() {
      return props.get("pointer-events") ?? "";
    },
    removeProperty(name: string) {
      props.delete(name);
      return "";
    },
  };
  return {
    style,
    closest(selector: string) {
      return selector.includes("data-mobile-code-layer") ? this : null;
    },
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
  } as unknown as HTMLElement;
}

Deno.test("code swipe freeze hides paint and leaves overflow alone", () => {
  const layer = fakeElement();
  hideMobileCodePaint(layer);
  assertEquals(layer.style.visibility, "hidden");
  assertEquals(layer.style.pointerEvents, "none");
  assertEquals(
    Object.getOwnPropertyDescriptor(layer.style, "overflow")?.value,
    undefined,
  );
  showMobileCodePaint(layer);
  assertEquals(layer.style.visibility, "");
});

Deno.test("code swipe freeze is reference-counted and skips after release", () => {
  const first = {
    scrollDOM: fakeElement(),
    dom: fakeElement(),
  };
  const second = {
    scrollDOM: fakeElement(),
    dom: fakeElement(),
  };
  const disposeFirst = bindCodeViewerSwipeFreeze(first, () => false);
  const disposeSecond = bindCodeViewerSwipeFreeze(second, () => false);
  assertEquals(isMobileCodeSwipeFrozen(), false);
  globalThis.dispatchEvent(new Event(MOBILE_CODE_SWIPE_START));
  assert(isMobileCodeSwipeFrozen());
  assertEquals(first.dom.getAttribute("data-mobile-code-frozen"), "true");
  assertEquals(second.dom.style.visibility, "hidden");
  globalThis.dispatchEvent(new Event(MOBILE_CODE_SWIPE_END));
  assertEquals(isMobileCodeSwipeFrozen(), false);
  assertEquals(first.dom.hasAttribute("data-mobile-code-frozen"), false);
  disposeFirst();
  disposeSecond();
});

Deno.test("a late-mounted editor freezes when a swipe is already claimed", () => {
  assert(
    swipeOwnsCodeSurface((selector) =>
      selector.includes("data-mobile-product-moving") ? {} : null
    ),
  );
  const view = {
    scrollDOM: fakeElement(),
    dom: fakeElement(),
  };
  const dispose = bindCodeViewerSwipeFreeze(view, () => true);
  assert(isMobileCodeSwipeFrozen());
  assertEquals(view.dom.style.visibility, "hidden");
  dispose();
  assertEquals(isMobileCodeSwipeFrozen(), false);
});
