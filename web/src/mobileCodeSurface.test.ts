import { assert, assertEquals } from "jsr:@std/assert";
import {
  bindCodeViewerSwipeFreeze,
  freezeMobileOverflowTile,
  isMobileCodeSwipeFrozen,
  MOBILE_CODE_SWIPE_END,
  MOBILE_CODE_SWIPE_START,
  swipeOwnsCodeSurface,
  thawMobileOverflowTile,
} from "./mobileCodeSurface.ts";

function fakeElement(): HTMLElement {
  const attrs = new Map<string, string>();
  const props = new Map<string, { value: string; priority: string }>();
  const style = {
    setProperty(name: string, value: string, priority = "") {
      props.set(name, { value, priority });
    },
    removeProperty(name: string) {
      props.delete(name);
      return "";
    },
    getPropertyPriority(name: string) {
      return props.get(name)?.priority ?? "";
    },
    get overflow() {
      return props.get("overflow")?.value ?? "";
    },
    get pointerEvents() {
      return props.get("pointer-events")?.value ?? "";
    },
  };
  return {
    style,
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

Deno.test("inline overflow flatten wins with important flags", () => {
  const scroller = fakeElement();
  freezeMobileOverflowTile(scroller);
  assertEquals(scroller.style.getPropertyPriority("overflow"), "important");
  assertEquals(scroller.style.getPropertyPriority("overflow-x"), "important");
  assertEquals(
    scroller.style.getPropertyPriority("-webkit-overflow-scrolling"),
    "important",
  );
  assertEquals(scroller.style.overflow, "hidden");
  assertEquals(scroller.style.pointerEvents, "none");
  thawMobileOverflowTile(scroller);
  assertEquals(scroller.style.overflow, "");
  assertEquals(scroller.style.pointerEvents, "");
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
  assertEquals(second.scrollDOM.style.overflow, "hidden");
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
  assertEquals(view.scrollDOM.style.overflow, "hidden");
  dispose();
  assertEquals(isMobileCodeSwipeFrozen(), false);
});
