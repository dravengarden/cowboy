import { assertEquals } from "jsr:@std/assert";
import type { CodeDocumentSymbol } from "./codeApi.ts";
import {
  activeOutlineRow,
  filterOutline,
  flattenOutline,
  symbolKindLabel,
} from "./outlineModel.ts";

const point = (row: number) => ({ row, column: 0 });
const symbol = (
  name: string,
  kind: number,
  start: number,
  end: number,
  children: CodeDocumentSymbol[] = [],
): CodeDocumentSymbol => ({
  name,
  kind,
  start: point(start),
  end: point(end),
  selectionStart: point(start),
  selectionEnd: point(start),
  children,
});

Deno.test("outline preserves hierarchy and supports contextual search", () => {
  const rows = flattenOutline([
    symbol("Judge", 22, 0, 20, [symbol("complete", 5, 5, 10)]),
    symbol("output_schema", 11, 22, 30),
  ]);
  assertEquals(rows.map(({ symbol, depth }) => [symbol.name, depth]), [
    ["Judge", 0],
    ["complete", 1],
    ["output_schema", 0],
  ]);
  assertEquals(
    filterOutline(rows, "judge complete").map((row) => row.symbol.name),
    ["complete"],
  );
  assertEquals(symbolKindLabel(11), "fn");
});

Deno.test("outline selects the deepest symbol containing the reading line", () => {
  const rows = flattenOutline([
    symbol("Judge", 22, 0, 20, [symbol("complete", 5, 5, 10)]),
  ]);
  assertEquals(activeOutlineRow(rows, 8)?.symbol.name, "complete");
});
