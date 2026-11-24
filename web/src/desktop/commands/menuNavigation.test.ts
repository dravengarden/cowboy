import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { desktopMenuNavigationKey } from "./menuNavigation.ts";

class FakeMenuElement {
  readonly selectors: Set<string>;
  parent: FakeMenuElement | null = null;
  readonly dispatched: string[] = [];
  constructor(selectors: string[]) {
    this.selectors = new Set(selectors);
  }
  matches(selector: string): boolean {
    return selector.split(",").some((part) => this.selectors.has(part.trim()));
  }
  closest(selector: string): FakeMenuElement | null {
    if (this.matches(selector)) return this;
    let cur: FakeMenuElement | null = this.parent;
    while (cur) {
      if (cur.matches(selector)) return cur;
      cur = cur.parent;
    }
    return null;
  }
  contains(node: FakeMenuElement): boolean {
    let cur: FakeMenuElement | null = node;
    while (cur) {
      if (cur === this) return true;
      cur = cur.parent;
    }
    return false;
  }
  getAttribute(): string | null {
    return null;
  }
  dispatchEvent(event: { key?: string }): boolean {
    if (event.key) this.dispatched.push(event.key);
    return true;
  }
}

function withFakeMenuDom<T>(fn: () => T): T {
  const g = globalThis as Record<string, unknown>;
  const saved = { Element: g.Element, HTMLElement: g.HTMLElement, KeyboardEvent: g.KeyboardEvent };
  g.Element = FakeMenuElement;
  g.HTMLElement = FakeMenuElement;
  g.KeyboardEvent = class FakeKeyboardEvent {
    readonly key: string;
    readonly code: string;
    constructor(_type: string, init: { key: string; code: string }) {
      this.key = init.key;
      this.code = init.code;
    }
  };
  try {
    return fn();
  } finally {
    Object.assign(g, saved);
  }
}

function makeKey(target: FakeMenuElement, key: string, code: string) {
  let prevented = false;
  const event = {
    key,
    code,
    keyCode: key.toUpperCase().charCodeAt(0),
    isComposing: false,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
    target,
    get defaultPrevented() {
      return prevented;
    },
    preventDefault() {
      prevented = true;
    },
    stopPropagation() {},
  };
  return event as unknown as KeyboardEvent;
}

test("desktopMenuNavigationKey stands aside for listboxes inside self-managed workbenches", () => {
  withFakeMenuDom(() => {
    const wrapper = new FakeMenuElement(["[data-desktop-keys='own']"]);
    const listbox = new FakeMenuElement(["[role='listbox']"]);
    const option = new FakeMenuElement(["[role='option']"]);
    listbox.parent = wrapper;
    option.parent = listbox;
    const event = makeKey(option, "j", "KeyJ");
    assertEquals(desktopMenuNavigationKey(event), false);
    assertEquals(event.defaultPrevented, false);
    assertEquals(option.dispatched, []);
  });
});

test("desktopMenuNavigationKey still translates j to ArrowDown for ordinary menus", () => {
  withFakeMenuDom(() => {
    const menu = new FakeMenuElement(["[role='menu']"]);
    const item = new FakeMenuElement(["[role='menuitem']"]);
    item.parent = menu;
    const event = makeKey(item, "j", "KeyJ");
    assertEquals(desktopMenuNavigationKey(event), true);
    assertEquals(item.dispatched, ["ArrowDown"]);
  });
});
