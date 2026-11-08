import { test } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { assert } from "../../../tools/test-assert.ts";
import {
  approval,
  comparison,
  hostileChoice,
  statusTable,
} from "../../../contracts/widget-v1.examples.ts";
import { parseWidgetSource } from "./widgetDocument";
import WidgetView from "./WidgetView";

function render(document: unknown): string {
  const parsed = parseWidgetSource(JSON.stringify(document));
  if (!parsed.ok) throw new Error(parsed.error);
  return renderToStaticMarkup(createElement(WidgetView, { document: parsed.document }));
}

test("the status table renders its rows, formatted bytes and an inert button", () => {
  const html = render(statusTable);
  assert(html.includes("Hawk services"));
  assert(html.includes("chrome-debug.service"));
  assert(html.includes("2.1 GB"), "bytes format");
  assert(html.includes('data-widget-table="table"'));
  assert(/<button[^>]*disabled/.test(html), "inline buttons are inert");
});

test("the approval card renders its kv facts, steps and status", () => {
  const html = render(approval);
  assert(html.includes("1b9f121eb"));
  assert(html.includes('data-step-state="done"'));
  assert(html.includes('data-step-state="pending"'));
  assert(html.includes("built"));
});

test("choices render their question and option labels, never option values", () => {
  const html = render(hostileChoice);
  assert(html.includes("Which cache may be removed?"));
  assert(html.includes("The stale Cargo target directory"));
  assert(!html.includes("rm_-rf_everything_now"));
  assert(render(comparison).includes("Go with A"));
});

test("images in the inline carrier show their alt text, never a remote fetch", () => {
  const html = render({
    schema: "cowboy.widget/1",
    title: "Shot",
    root: {
      type: "image",
      alt: "Login page at 375px",
      src: { kind: "artifact", url: "/api/artifacts/x.png" },
    },
  });
  assert(html.includes("Login page at 375px"));
  assert(!html.includes("<img"));
});

test("unknown components render their fallback text", () => {
  const html = render({
    schema: "cowboy.widget/1",
    title: "Future",
    root: { type: "stack", children: [{ type: "sparkline" }] },
  });
  assert(html.includes("Unsupported widget component: sparkline"));
});
