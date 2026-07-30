import type { CodeDocumentSymbol } from "./codeApi";

export interface OutlineRow {
  symbol: CodeDocumentSymbol;
  depth: number;
  context: string;
}

export function symbolKindLabel(kind: number): string {
  return [
    "file",
    "mod",
    "namespace",
    "package",
    "class",
    "method",
    "property",
    "field",
    "ctor",
    "enum",
    "interface",
    "fn",
    "var",
    "const",
    "string",
    "number",
    "bool",
    "array",
    "object",
    "key",
    "null",
    "member",
    "struct",
    "event",
    "operator",
    "type",
  ][kind] ?? "symbol";
}

export function flattenOutline(
  symbols: readonly CodeDocumentSymbol[],
  depth = 0,
  ancestors: readonly string[] = [],
): OutlineRow[] {
  return symbols.flatMap((symbol) => {
    const context = [...ancestors, symbol.name].join(" ");
    return [
      { symbol, depth, context },
      ...flattenOutline(symbol.children, depth + 1, [
        ...ancestors,
        symbol.name,
      ]),
    ];
  });
}

export function filterOutline(
  rows: readonly OutlineRow[],
  query: string,
): OutlineRow[] {
  const terms = query.toLocaleLowerCase().trim().split(/\s+/u).filter(Boolean);
  if (terms.length === 0) return [...rows];
  return rows.filter(({ symbol, context }) => {
    const haystack = `${symbolKindLabel(symbol.kind)} ${context}`
      .toLocaleLowerCase();
    return terms.every((term) => haystack.includes(term));
  });
}

export function activeOutlineRow(
  rows: readonly OutlineRow[],
  line: number | undefined,
): OutlineRow | undefined {
  if (line === undefined) return undefined;
  return rows
    .filter(({ symbol }) =>
      symbol.start.row + 1 <= line && symbol.end.row + 1 >= line
    )
    .sort((left, right) => right.depth - left.depth)[0];
}
