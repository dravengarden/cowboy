import { test } from "bun:test";
import {
  approval,
  approvalMarkdown,
  comparison,
  comparisonMarkdown,
  progressMarkdown,
  progressPatch,
  statusTable,
  statusTableMarkdown,
} from "./widget-v1.examples.ts";
import { type Node, WIDGET_ID_PATTERN, WIDGET_LIMITS, type WidgetDocument } from "./widget-v1.ts";

const bytes = (value: unknown): number => new TextEncoder().encode(JSON.stringify(value)).length;

function children(node: Node): Node[] {
  switch (node.type) {
    case "stack":
    case "grid":
    case "section":
      return node.children;
    case "card":
      return [...node.children, ...(node.actions ?? [])];
    case "tabs":
      return node.tabs.flatMap((tab) => tab.children);
    default:
      return [];
  }
}

/** Walks a document the way both validators must: bounded nodes, depth,
 *  unique ids, and the per-type collection limits. */
function check(document: WidgetDocument): { nodes: number; depth: number } {
  const ids = new Set<string>();
  let nodes = 0;
  let deepest = 0;
  const visit = (node: Node, depth: number): void => {
    nodes += 1;
    deepest = Math.max(deepest, depth);
    if (node.id !== undefined) {
      if (!WIDGET_ID_PATTERN.test(node.id) || ids.has(node.id))
        throw new Error(`bad id ${node.id}`);
      ids.add(node.id);
    }
    if (node.type === "table") {
      if (node.columns.length > WIDGET_LIMITS.tableColumns) throw new Error("too many columns");
      if (node.rows.length > WIDGET_LIMITS.tableRows) throw new Error("too many rows");
      for (const row of node.rows) {
        if (row.length !== node.columns.length) throw new Error("ragged row");
      }
    }
    if (node.type === "card" && (node.actions?.length ?? 0) > WIDGET_LIMITS.cardActions) {
      throw new Error("too many card actions");
    }
    for (const child of children(node)) visit(child, depth + 1);
  };
  visit(document.root, 1);
  if (nodes > WIDGET_LIMITS.nodes) throw new Error("too many nodes");
  if (deepest > WIDGET_LIMITS.depth) throw new Error("too deep");
  if (document.title.length > WIDGET_LIMITS.titleChars) throw new Error("title too long");
  if ((document.summary?.length ?? 0) > WIDGET_LIMITS.summaryChars)
    throw new Error("summary too long");
  if (bytes(document) > WIDGET_LIMITS.documentBytes) throw new Error("document too large");
  return { nodes, depth: deepest };
}

const examples: [string, WidgetDocument, string][] = [
  ["status table", statusTable, statusTableMarkdown],
  ["comparison", comparison, comparisonMarkdown],
  ["approval", approval, approvalMarkdown],
];

for (const [name, document, markdown] of examples) {
  test(`widget example is within bounds: ${name}`, () => {
    const { nodes, depth } = check(document);
    if (nodes < 2 || depth < 2) throw new Error(`example too trivial: ${name}`);
  });

  // docs/widgets.md §10 records how each example compares with the Markdown
  // an agent writes today. Bytes are a poor proxy across scripts, so this
  // only guards against the contract drifting into a shape that dwarfs the
  // prose it replaces; the measured ratios live in the design document.
  test(`widget example does not dwarf its Markdown: ${name}`, () => {
    const ratio = bytes(document) / new TextEncoder().encode(markdown).length;
    if (ratio > 4) throw new Error(`${name}: widget is ${ratio.toFixed(2)}× the Markdown`);
  });
}

test("a patch costs what it changes, not what it repeats", () => {
  const patch = bytes(progressPatch);
  if (patch > WIDGET_LIMITS.documentBytes / 8) throw new Error("patch example too large");
  if (progressPatch.length > WIDGET_LIMITS.patchOps) throw new Error("too many ops");
  // The equivalent re-sent status message is a floor, not a ceiling: the
  // patch must stay below re-sending the approval card it updates.
  if (patch >= bytes(approval)) throw new Error("patch is not cheaper than a re-send");
  if (new TextEncoder().encode(progressMarkdown).length === 0) throw new Error("missing baseline");
});
