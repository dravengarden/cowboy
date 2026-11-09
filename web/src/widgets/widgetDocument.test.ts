import { test } from "bun:test";
import { assert, assertEquals } from "../../../tools/test-assert.ts";
import { approval, comparison, statusTable } from "../../../contracts/widget-v1.examples.ts";
import { parseWidgetSource, parseWidgetValue } from "./widgetDocument";

test("the contract examples parse", () => {
  for (const document of [statusTable, comparison, approval]) {
    const result = parseWidgetSource(JSON.stringify(document));
    assert(result.ok, result.ok ? "" : result.error);
  }
});

test("an unknown node type becomes an unsupported node with its fallback", () => {
  const result = parseWidgetSource(
    JSON.stringify({
      schema: "cowboy.widget/1",
      title: "Future",
      root: {
        type: "stack",
        children: [{ type: "chart", fallback: "a chart" }, { type: "divider" }],
      },
    }),
  );
  assert(result.ok);
  if (!result.ok || result.document.root.type !== "stack") throw new Error("unexpected");
  assertEquals(result.document.root.children[0], {
    type: "unsupported",
    original: "chart",
    fallback: "a chart",
  });
});

test("shape and bound violations reject the document", () => {
  const base = { schema: "cowboy.widget/1", title: "T" };
  const cases: [string, unknown][] = [
    ["wrong schema", { ...base, schema: "cowboy.widget/2", root: { type: "divider" } }],
    ["ragged table", { ...base, root: { type: "table", columns: ["a", "b"], rows: [["1"]] } }],
    ["multi-line title", { ...base, title: "a\nb", root: { type: "divider" } }],
    [
      "duplicate id",
      {
        ...base,
        root: {
          type: "stack",
          children: [
            { type: "divider", id: "x" },
            { type: "divider", id: "x" },
          ],
        },
      },
    ],
    [
      "prose option value",
      {
        ...base,
        root: { type: "choice", id: "c", label: "Q", options: [{ value: "rm -rf", label: "A" }] },
      },
    ],
    ["progress out of range", { ...base, root: { type: "progress", value: 2 } }],
    [
      "card action not a button",
      { ...base, root: { type: "card", children: [], actions: [{ type: "divider" }] } },
    ],
  ];
  for (const [name, document] of cases) {
    const result = parseWidgetSource(JSON.stringify(document));
    assert(!result.ok, `accepted: ${name}`);
  }
  const duplicateKey = parseWidgetSource(
    '{"schema":"cowboy.widget/1","title":"a","title":"b","root":{"type":"divider"}}',
  );
  assert(!duplicateKey.ok, "duplicate keys accepted");
  assert(!parseWidgetSource("{not json").ok);
});

test("the aggregate cell budget is enforced across tables", () => {
  const table = {
    type: "table",
    columns: Array.from({ length: 12 }, (_, i) => `c${i}`),
    rows: Array.from({ length: 200 }, () => Array.from({ length: 12 }, () => 1)),
  };
  const result = parseWidgetSource(
    JSON.stringify({
      schema: "cowboy.widget/1",
      title: "T",
      root: { type: "stack", children: [table] },
    }),
  );
  assert(!result.ok, "2,400 cells accepted");
});

test("a button's action fields are checked before any renderer reads them", () => {
  for (const action of [{ kind: "open" }, { kind: "open", to: { kind: "url" } }, { kind: "copy" }]) {
    const parsed = parseWidgetValue({
      schema: "cowboy.widget/1",
      title: "x",
      root: { type: "button", id: "b", label: "Go", action },
    });
    assert(!parsed.ok, JSON.stringify(action));
  }
});
