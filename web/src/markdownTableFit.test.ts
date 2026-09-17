import { assert, assertEquals } from "jsr:@std/assert";
import {
  fitMarkdownTables,
  MARKDOWN_TABLE_WRAP_ATTRIBUTE,
  markdownTableFitSx,
  markdownTableOverflows,
} from "./markdownTableFit.ts";

const markdownImpl = await Deno.readTextFile(
  new URL("./MarkdownImpl.tsx", import.meta.url),
);

/** A table whose laid-out width depends on its wrap step. */
function fakeTable(widths: { words: number; compact: number; anywhere: number }) {
  const attributes = new Map<string, string>();
  const writes: string[] = [];
  return {
    writes,
    attributes,
    parentElement: { clientWidth: 354 },
    removeAttribute(name: string) {
      writes.push(`remove:${name}`);
      attributes.delete(name);
    },
    setAttribute(name: string, value: string) {
      writes.push(`set:${value}`);
      attributes.set(name, value);
    },
    get offsetWidth() {
      const wrap = attributes.get(MARKDOWN_TABLE_WRAP_ATTRIBUTE);
      return wrap === "anywhere"
        ? widths.anywhere
        : wrap === "compact"
        ? widths.compact
        : widths.words;
    },
  };
}

Deno.test("a table overflows only past the one-pixel rounding tolerance", () => {
  assertEquals(markdownTableOverflows(355, 354), false);
  assertEquals(markdownTableOverflows(355.5, 354), true);
});

Deno.test("tables step from word wrap to compact before breaking anywhere", () => {
  const fits = fakeTable({ words: 300, compact: 280, anywhere: 250 });
  const compact = fakeTable({ words: 391, compact: 354, anywhere: 330 });
  const anywhere = fakeTable({ words: 449, compact: 392, anywhere: 354 });
  fitMarkdownTables([fits, compact, anywhere]);
  assertEquals(fits.attributes.get(MARKDOWN_TABLE_WRAP_ATTRIBUTE), undefined);
  assertEquals(compact.attributes.get(MARKDOWN_TABLE_WRAP_ATTRIBUTE), "compact");
  assertEquals(anywhere.attributes.get(MARKDOWN_TABLE_WRAP_ATTRIBUTE), "anywhere");
});

Deno.test("a refit re-evaluates from word wrap after the column widens", () => {
  const table = fakeTable({ words: 449, compact: 392, anywhere: 354 });
  fitMarkdownTables([table]);
  assertEquals(table.attributes.get(MARKDOWN_TABLE_WRAP_ATTRIBUTE), "anywhere");
  table.parentElement.clientWidth = 600;
  fitMarkdownTables([table]);
  assertEquals(table.attributes.get(MARKDOWN_TABLE_WRAP_ATTRIBUTE), undefined);
});

Deno.test("fit mode never leaves a table wrapper as a scroll container", () => {
  const wrapper = markdownTableFitSx["& [data-markdown-table-scroll]"];
  assertEquals(wrapper.overflowX, "clip");
  assertEquals(wrapper.WebkitOverflowScrolling, "auto");
  assertEquals(
    markdownTableFitSx["& [data-markdown-table-scroll] > table :is(th, td)"]
      .whiteSpace,
    "normal",
  );
});

Deno.test("only touch-wrapped Markdown on a coarse pointer fits its tables", () => {
  assert(markdownImpl.includes("const fitTables = touchWrap && coarse;"));
  assert(markdownImpl.includes("{ noSsr: true }"));
  assert(markdownImpl.includes("...(fitTables && markdownTableFitSx)"));
  assert(markdownImpl.includes("return bindMarkdownTableFit(root);"));
  // The shared table renderer keeps its native scroller everywhere else, and
  // its identity stays module-scoped (see MarkdownTable).
  assert(markdownImpl.includes('overflowX: "auto"'));
  assert(markdownImpl.includes("table: MarkdownTable,"));
});
