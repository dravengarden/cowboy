import { assertEquals } from "jsr:@std/assert";
import remarkLineBreakTags from "./markdownLineBreaks.ts";

Deno.test("a <br> tag in a table cell becomes a hard break", () => {
  const cell = {
    type: "tableCell",
    children: [
      { type: "text", value: "a" },
      { type: "html", value: "<br>" },
      { type: "text", value: "b" },
      { type: "html", value: "<BR />" },
    ],
  };
  const tree = {
    type: "root",
    children: [{ type: "table", children: [cell] }],
  };
  remarkLineBreakTags()(tree);
  assertEquals(cell.children.map((node) => node.type), [
    "text",
    "break",
    "text",
    "break",
  ]);
});

Deno.test("other raw HTML stays escaped text", () => {
  const html = { type: "html", value: "<script>x</script>" };
  const tree = {
    type: "root",
    children: [{ type: "paragraph", children: [html] }],
  };
  remarkLineBreakTags()(tree);
  assertEquals(html.type, "html");
});
