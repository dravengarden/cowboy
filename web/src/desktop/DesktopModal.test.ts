import { test } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { assert, assertEquals } from "../../../tools/test-assert.ts";
import { handleIdleCjkDialogEscape, InstantModalTransition } from "./DesktopModal.tsx";

test("InstantModalTransition forwards MUI Modal tabIndex and onFocus without leaking transition props", () => {
  const child = createElement("div", { "data-paper": "true" }, "content");
  const el = createElement(InstantModalTransition as unknown as React.ComponentType<Record<string, unknown>>, {
    in: true,
    tabIndex: -1,
    appear: true,
    timeout: 225,
    onFocus: () => {},
    children: child,
  });
  const html = renderToStaticMarkup(el);
  assert(html.includes('tabindex="-1"'), `expected tabindex="-1" in ${html}`);
  assertEquals(html.includes("appear"), false);
  assertEquals(html.includes("timeout"), false);
});

test("handleIdleCjkDialogEscape closes topmost MUI Dialog root on idle CJK Escape only", () => {
  const g = globalThis as Record<string, unknown>;
  const saved = { document: g.document, getComputedStyle: g.getComputedStyle };
  const makeDialog = () => {
    const paper = {
      getClientRects: () => [{ width: 400, height: 300 }],
      closest: (s: string) => (s === "[role='dialog'][aria-modal='true']" ? paper : null),
    };
    const root = {
      closest: () => null,
      querySelector: (s: string) => (s === "[role='dialog'][aria-modal='true']" ? paper : null),
    };
    return { root, paper };
  };
  const outer = makeDialog();
  const inner = makeDialog();
  let openDialogs: unknown[] = [outer.paper];
  g.getComputedStyle = () => ({ visibility: "visible", display: "block" });
  g.document = {
    querySelector: () => null,
    querySelectorAll: (s: string) =>
      s === "[role='dialog'][aria-modal='true']" ? openDialogs : [],
  };
  try {
    const makeEvent = (
      currentTarget: unknown,
      overrides: {
        key?: string;
        code?: string;
        keyCode?: number;
        isComposing?: boolean;
        defaultPrevented?: boolean;
      } = {},
    ) => {
      let prevented = Boolean(overrides.defaultPrevented);
      let stopped = false;
      const nativeEvent = {
        key: overrides.key ?? "Process",
        code: overrides.code ?? "Escape",
        keyCode: overrides.keyCode ?? 229,
        isComposing: Boolean(overrides.isComposing),
        defaultPrevented: prevented,
        target: null,
      };
      return {
        key: nativeEvent.key,
        nativeEvent,
        currentTarget,
        get defaultPrevented() {
          return prevented;
        },
        preventDefault() {
          prevented = true;
        },
        stopPropagation() {
          stopped = true;
        },
        get stopped() {
          return stopped;
        },
      };
    };

    // Idle CJK Process/Escape closes the topmost MuiDialog-root wrapper.
    let closed = 0;
    const ev1 = makeEvent(outer.root);
    handleIdleCjkDialogEscape(ev1 as never, () => { closed += 1; });
    assertEquals(closed, 1);
    assertEquals(ev1.defaultPrevented, true);
    assertEquals(ev1.stopped, true);

    // Ordinary Escape is left for MUI Dialog onClose (no double close).
    const ev2 = makeEvent(outer.root, { key: "Escape", keyCode: 27 });
    handleIdleCjkDialogEscape(ev2 as never, () => { closed += 1; });
    assertEquals(closed, 1);

    // Active IME composition does not close the dialog.
    const ev3 = makeEvent(outer.root, { isComposing: true });
    handleIdleCjkDialogEscape(ev3 as never, () => { closed += 1; });
    assertEquals(closed, 1);

    // Already defaultPrevented (e.g. inner inputVim Escape) does not close.
    const ev4 = makeEvent(outer.root, { defaultPrevented: true });
    handleIdleCjkDialogEscape(ev4 as never, () => { closed += 1; });
    assertEquals(closed, 1);

    // Non-topmost dialog does not close when a nested dialog sits above it.
    openDialogs = [outer.paper, inner.paper];
    const ev5 = makeEvent(outer.root);
    handleIdleCjkDialogEscape(ev5 as never, () => { closed += 1; });
    assertEquals(closed, 1);

    // Topmost nested dialog does close.
    const ev6 = makeEvent(inner.root);
    handleIdleCjkDialogEscape(ev6 as never, () => { closed += 1; });
    assertEquals(closed, 2);
  } finally {
    Object.assign(g, saved);
  }
});
