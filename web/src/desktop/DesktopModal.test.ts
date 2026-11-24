import { test } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { assert, assertEquals } from "../../../tools/test-assert.ts";
import { InstantModalTransition } from "./DesktopModal.tsx";

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
