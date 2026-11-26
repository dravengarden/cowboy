import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  type BackgroundTasksKeyContext,
  handleBackgroundTasksKeyDown,
} from "./backgroundTasksKeyboard.ts";

class FakeNode {
  parent: FakeNode | null = null;
  readonly children: FakeNode[] = [];
  readonly selectors: Set<string>;
  readonly isConnected = true;
  constructor(selectors: string[] = []) {
    this.selectors = new Set(selectors);
  }
  append(child: FakeNode): FakeNode {
    child.parent = this;
    this.children.push(child);
    return child;
  }
  contains(node: unknown): boolean {
    if (node === this) return true;
    return this.children.some((child) => child.contains(node));
  }
  matches(selector: string): boolean {
    return selector.split(",").some((part) => this.selectors.has(part.trim()));
  }
  closest(selector: string): FakeNode | null {
    if (this.matches(selector)) return this;
    return this.parent?.closest(selector) ?? null;
  }
  getAttribute(): string | null {
    return null;
  }
  hasAttribute(): boolean {
    return false;
  }
  getClientRects() {
    return [{ width: 800, height: 600 }];
  }
}

class FakeInput extends FakeNode {
  type = "text";
  readOnly = false;
  constructor() {
    super(["input"]);
  }
}

function makeEvent(
  target: unknown,
  overrides: Partial<KeyboardEvent> = {},
) {
  let prevented = false;
  let stopped = false;
  const event = {
    key: "j",
    code: "KeyJ",
    keyCode: 74,
    isComposing: false,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    target,
    preventDefault() {
      prevented = true;
    },
    stopPropagation() {
      stopped = true;
    },
    ...overrides,
  };
  return {
    event: event as unknown as KeyboardEvent,
    get prevented() {
      return prevented;
    },
    get stopped() {
      return stopped;
    },
  };
}

test("background tasks keyboard handles body focus, leaked background inputs, and idle CJK IME", () => {
  const g = globalThis as Record<string, unknown>;
  const saved = {
    Node: g.Node,
    Element: g.Element,
    HTMLElement: g.HTMLElement,
    HTMLInputElement: g.HTMLInputElement,
    HTMLTextAreaElement: g.HTMLTextAreaElement,
    HTMLSelectElement: g.HTMLSelectElement,
    document: g.document,
    getComputedStyle: g.getComputedStyle,
  };
  const body = new FakeNode(["body"]);
  const bgInput = body.append(new FakeInput());
  const dialog = body.append(new FakeNode(["[role='dialog'][aria-modal='true']", "[role='dialog']"]));
  const inspector = dialog.append(new FakeNode(["div"]));
  const nestedDialog = body.append(new FakeNode(["[role='dialog'][aria-modal='true']", "[role='dialog']"]));
  let openDialogs: FakeNode[] = [dialog];
  g.Node = FakeNode;
  g.Element = FakeNode;
  g.HTMLElement = FakeNode;
  g.HTMLInputElement = FakeInput;
  g.HTMLTextAreaElement = class extends FakeNode {};
  g.HTMLSelectElement = class extends FakeNode {};
  g.getComputedStyle = () => ({ visibility: "visible", display: "block" });
  g.document = {
    activeElement: body,
    querySelector: () => null,
    querySelectorAll: (selector: string) =>
      selector === "[role='dialog'][aria-modal='true']" ? openDialogs : [],
  };

  const log: string[] = [];
  const ctx: BackgroundTasksKeyContext = {
    ownNode: inspector as unknown as HTMLElement,
    selected: { key: "call:1", source: "call", anchorSeq: 5, canStop: true },
    step: (delta) => log.push(`step:${String(delta)}`),
    openConversation: () => log.push("open"),
    locate: (key) => log.push(`locate:${key}`),
    answerApproval: () => log.push("approval"),
    close: () => log.push("close"),
  };

  try {
    // Body focus (e.g. after a task button unmounts) still drives shortcuts.
    const e1 = makeEvent(body, { key: "j", code: "KeyJ" });
    handleBackgroundTasksKeyDown(e1.event, ctx);
    assertEquals(log, ["step:1"]);
    assertEquals(e1.prevented && e1.stopped, true);

    // Idle CJK IME on modal chrome resolves physically, including full-width brackets and Shift+L.
    for (const overrides of [
      { key: "Process", code: "KeyK", keyCode: 229 },
      { key: "】", code: "BracketRight", keyCode: 229 },
      { key: "【", code: "", keyCode: 219 },
      { key: "Process", code: "KeyO", keyCode: 229 },
      { key: "Process", code: "KeyL", shiftKey: true, keyCode: 229 },
      { key: "Process", code: "Escape", keyCode: 229 },
    ]) {
      handleBackgroundTasksKeyDown(makeEvent(inspector, overrides).event, ctx);
    }
    assertEquals(log, [
      "step:1",
      "step:-1",
      "step:1",
      "step:-1",
      "open",
      "locate:call:1",
      "close",
    ]);

    // Idle CJK marker on an underlying background input behind the modal still works.
    log.length = 0;
    handleBackgroundTasksKeyDown(
      makeEvent(bgInput, { key: "Process", code: "KeyJ", keyCode: 229 }).event,
      ctx,
    );
    assertEquals(log, ["step:1"]);

    // Active IME composition never triggers shortcuts.
    log.length = 0;
    handleBackgroundTasksKeyDown(
      makeEvent(inspector, { key: "Process", code: "Escape", keyCode: 229, isComposing: true }).event,
      ctx,
    );
    assertEquals(log, []);

    // When a nested dialog sits on top, BackgroundTasksInspector stands aside.
    openDialogs = [dialog, nestedDialog];
    handleBackgroundTasksKeyDown(
      makeEvent(nestedDialog, { key: "Escape", code: "Escape", keyCode: 27 }).event,
      ctx,
    );
    assertEquals(log, []);
  } finally {
    Object.assign(g, saved);
  }
});
