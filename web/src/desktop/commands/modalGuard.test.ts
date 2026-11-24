import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  focusAdjacentOutsideRegion,
  isInteractiveControl,
  isModalEditableTarget,
  isTopmostDialog,
  topmostDialog,
} from "./modalGuard.ts";

class FakeDomElement {
  readonly attrs = new Map<string, string>();
  readonly selectors: Set<string>;
  parent: FakeDomElement | null = null;
  constructor(selectors: string[] = []) {
    this.selectors = new Set(selectors);
  }
  getAttribute(name: string): string | null {
    return this.attrs.get(name) ?? null;
  }
  setAttribute(name: string, value: string): void {
    this.attrs.set(name, value);
  }
  matches(selector: string): boolean {
    return selector.split(",").some((part) => this.selectors.has(part.trim()));
  }
  closest(selector: string): FakeDomElement | null {
    if (this.matches(selector)) return this;
    let cur: FakeDomElement | null = this.parent;
    while (cur) {
      if (cur.matches(selector)) return cur;
      cur = cur.parent;
    }
    return null;
  }
}

class FakeInput extends FakeDomElement {
  type = "text";
  readOnly = false;
  constructor(type = "text") {
    super(["input"]);
    this.type = type;
  }
}

class FakeTextArea extends FakeDomElement {
  readOnly = false;
  constructor() {
    super(["textarea"]);
  }
}

class FakeSelect extends FakeDomElement {}

function withDomGlobals<T>(fn: () => T): T {
  const g = globalThis as Record<string, unknown>;
  const saved = {
    Element: g.Element,
    HTMLElement: g.HTMLElement,
    HTMLInputElement: g.HTMLInputElement,
    HTMLTextAreaElement: g.HTMLTextAreaElement,
    HTMLSelectElement: g.HTMLSelectElement,
    getComputedStyle: g.getComputedStyle,
  };
  g.Element = FakeDomElement;
  g.HTMLElement = FakeDomElement;
  g.HTMLInputElement = FakeInput;
  g.HTMLTextAreaElement = FakeTextArea;
  g.HTMLSelectElement = FakeSelect;
  g.getComputedStyle = () => ({ visibility: "visible", display: "block" });
  try {
    return fn();
  } finally {
    Object.assign(g, saved);
  }
}

function fakeDialog(visible = true): HTMLElement {
  const el = {
    getClientRects: () => (visible ? [{ width: 100, height: 100 }] : []),
    closest(selector: string) {
      return selector === "[role='dialog'][aria-modal='true']" ? el : null;
    },
  };
  return el as unknown as HTMLElement;
}

test("isTopmostDialog compares owning modal dialog roots, ignoring nested layers beneath", () => {
  withDomGlobals(() => {
    const outer = fakeDialog(true);
    const inner = fakeDialog(true);
    const childOfOuter = {
      closest: (selector: string) =>
        selector === "[role='dialog'][aria-modal='true']" ? outer : null,
    } as unknown as Element;
    const childOfInner = {
      closest: (selector: string) =>
        selector === "[role='dialog'][aria-modal='true']" ? inner : null,
    } as unknown as Element;
    const doc = {
      querySelectorAll: () => [outer, inner],
    } as unknown as Document;

    assertEquals(topmostDialog(doc), inner);
    assertEquals(isTopmostDialog(childOfOuter, doc), false);
    assertEquals(isTopmostDialog(childOfInner, doc), true);
  });
});

test("isModalEditableTarget treats inputVim Normal fields as non-editable command chrome", () => {
  withDomGlobals(() => {
    const input = new FakeInput("text");
    assertEquals(isModalEditableTarget(input as unknown as EventTarget), true);

    input.setAttribute("data-vim-input-mode", "normal");
    input.readOnly = true;
    assertEquals(isModalEditableTarget(input as unknown as EventTarget), false);

    const checkbox = new FakeInput("checkbox");
    assertEquals(isModalEditableTarget(checkbox as unknown as EventTarget), false);
  });
});

test("isInteractiveControl recognizes native buttons and links", () => {
  withDomGlobals(() => {
    const btn = new FakeDomElement(["button"]);
    const span = new FakeDomElement(["span"]);
    span.parent = btn;
    assertEquals(isInteractiveControl(span as unknown as EventTarget), true);
    assertEquals(isInteractiveControl(new FakeDomElement(["div"]) as unknown as EventTarget), false);
  });
});

test("focusAdjacentOutsideRegion skips tabindex=-1 controls such as status-line buttons", () => {
  withDomGlobals(() => {
    const g = globalThis as Record<string, unknown>;
    const savedDoc = g.document;
    const savedNode = g.Node;
    g.Node = {
      DOCUMENT_POSITION_PRECEDING: 2,
      DOCUMENT_POSITION_FOLLOWING: 4,
    };
    let focused: string | null = null;
    const makeBtn = (id: string, tabIndex: number, pos: number) => ({
      id,
      tabIndex,
      pos,
      matches: () => false,
      closest: () => null,
      getClientRects: () => [{ width: 40, height: 20 }],
      focus: () => { focused = id; },
    });
    const before = makeBtn("sessions-row", 0, 2);
    const statusBtn = makeBtn("status-commands", -1, 4);
    const composer = makeBtn("composer-input", 0, 4);
    const region = {
      contains: () => false,
      compareDocumentPosition: (el: { pos: number }) => el.pos,
    } as unknown as HTMLElement;
    g.document = {
      querySelectorAll: () => [before, statusBtn, composer],
    };
    try {
      assertEquals(focusAdjacentOutsideRegion(region, 1), true);
      assertEquals(focused, "composer-input");
      assertEquals(focusAdjacentOutsideRegion(region, -1), true);
      assertEquals(focused, "sessions-row");
    } finally {
      g.document = savedDoc;
      g.Node = savedNode;
    }
  });
});
