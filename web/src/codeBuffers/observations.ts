import {
  type Diagnostic,
  type DocumentSymbol,
  type InlayHint,
  integer,
  list,
  type Observation,
  type Point,
  type ReadKind,
  record,
  requireValue,
  type ResourceId,
  type Results,
  text,
} from "./protocol.ts";

function point(value: unknown): Point {
  const row = record(value, ["row", "column"]);
  return Object.freeze({ row: integer(row.row), column: integer(row.column) });
}
function ordered(start: Point, end: Point): boolean {
  return start.row < end.row ||
    (start.row === end.row && start.column <= end.column);
}
function nullableText(value: unknown): string | null {
  return value === null ? null : text(value);
}
function diagnostic(value: unknown): Diagnostic {
  const row = record(value, ["start", "end", "severity", "source", "message"]);
  const start = point(row.start), end = point(row.end);
  requireValue(ordered(start, end));
  return Object.freeze({
    start,
    end,
    severity: integer(row.severity, -0x8000_0000, 0x7fff_ffff),
    source: nullableText(row.source),
    message: text(row.message),
  });
}
function inlay(value: unknown): InlayHint {
  const row = record(value, [
    "offset",
    "label",
    "kind",
    "paddingLeft",
    "paddingRight",
  ]);
  requireValue(
    typeof row.paddingLeft === "boolean" &&
      typeof row.paddingRight === "boolean",
  );
  return Object.freeze({
    offset: integer(row.offset),
    label: text(row.label),
    kind: nullableText(row.kind),
    paddingLeft: row.paddingLeft,
    paddingRight: row.paddingRight,
  });
}
function language(value: unknown): Results["language"] {
  const row = record(value, [
    "kind",
    "diagnosticsState",
    "diagnostics",
    "inlayHints",
    "semanticTokens",
  ]);
  requireValue(
    row.kind === "language" &&
      (row.diagnosticsState === "observed" ||
        row.diagnosticsState === "unobserved"),
  );
  const diagnostics = Object.freeze(
    list(row.diagnostics, 1_000).map(diagnostic),
  );
  requireValue(row.diagnosticsState === "observed" || diagnostics.length === 0);
  const semanticTokens = Object.freeze(
    list(row.semanticTokens, 50_000).map((value) => integer(value)),
  );
  requireValue(semanticTokens.length % 5 === 0);
  return Object.freeze({
    kind: "language",
    diagnosticsState: row.diagnosticsState,
    diagnostics,
    inlayHints: Object.freeze(list(row.inlayHints, 2_000).map(inlay)),
    semanticTokens,
  });
}
function symbols(value: unknown): Results["symbols"] {
  const row = record(value, ["kind", "symbols"]);
  requireValue(row.kind === "symbols");
  let remaining = 2_000;
  const tree = (value: unknown, depth: number): DocumentSymbol => {
    requireValue(depth <= 16 && remaining-- > 0);
    const node = record(value, [
      "name",
      "kind",
      "start",
      "end",
      "selectionStart",
      "selectionEnd",
      "children",
    ]);
    const start = point(node.start), end = point(node.end);
    const selectionStart = point(node.selectionStart),
      selectionEnd = point(node.selectionEnd);
    requireValue(
      ordered(start, selectionStart) && ordered(selectionStart, selectionEnd) &&
        ordered(selectionEnd, end),
    );
    return Object.freeze({
      name: text(node.name),
      kind: integer(node.kind, -0x8000_0000, 0x7fff_ffff),
      start,
      end,
      selectionStart,
      selectionEnd,
      children: Object.freeze(
        list(node.children, remaining).map((child) => tree(child, depth + 1)),
      ),
    });
  };
  return Object.freeze({
    kind: "symbols",
    symbols: Object.freeze(
      list(row.symbols, remaining).map((node) => tree(node, 1)),
    ),
  });
}

export function decodeObservation<K extends ReadKind>(
  value: unknown,
  id: ResourceId,
  kind: K,
): Observation<K> {
  const row = record(value, [
    "apiVersion",
    "resourceId",
    "openedVersion",
    "result",
  ]);
  requireValue(row.apiVersion === 1 && row.resourceId === id);
  let previous = -1;
  const openedVersion = Object.freeze(
    list(row.openedVersion, 256).map((entry) => {
      const row = record(entry, ["replicaId", "timestamp"]);
      const replicaId = integer(row.replicaId);
      requireValue(replicaId > previous);
      previous = replicaId;
      return Object.freeze({ replicaId, timestamp: integer(row.timestamp) });
    }),
  );
  // The selected decoder independently checks the tag; no unchecked wire cast.
  const result = kind === "language"
    ? language(row.result)
    : symbols(row.result);
  requireValue(result.kind === kind);
  return Object.freeze({
    apiVersion: 1,
    resourceId: id,
    openedVersion,
    result,
  }) as Observation<K>;
}
