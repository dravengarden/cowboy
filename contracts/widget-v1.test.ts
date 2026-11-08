import { test } from "bun:test";
import {
  approval,
  approvalMarkdown,
  approvalTapMessage,
  comparison,
  comparisonMarkdown,
  progressMarkdown,
  progressPatch,
  statusTable,
  statusTableMarkdown,
} from "./widget-v1.examples.ts";
import { type Node, WIDGET_ID_PATTERN, WIDGET_LIMITS, type WidgetDocument } from "./widget-v1.ts";

const encoder = new TextEncoder();
const bytes = (value: unknown): number =>
  encoder.encode(typeof value === "string" ? value : JSON.stringify(value)).length;

function children(node: Node): Node[] {
  switch (node.type) {
    case "stack":
    case "section":
      return node.children;
    case "card":
      return [...node.children, ...(node.actions ?? [])];
    default:
      return [];
  }
}

/** Walks a document the way both validators must: bounded nodes, depth,
 *  unique ids, identifier-shaped option values, the per-type collection
 *  limits and the aggregate rendering budgets. */
function check(document: WidgetDocument): { nodes: number; depth: number } {
  const ids = new Set<string>();
  let nodes = 0;
  let deepest = 0;
  let cells = 0;
  let items = 0;
  let images = 0;
  const visit = (node: Node, depth: number): void => {
    nodes += 1;
    deepest = Math.max(deepest, depth);
    if (node.id !== undefined) {
      if (!WIDGET_ID_PATTERN.test(node.id) || ids.has(node.id))
        throw new Error(`bad id ${node.id}`);
      ids.add(node.id);
    }
    switch (node.type) {
      case "table":
        if (node.columns.length > WIDGET_LIMITS.tableColumns) throw new Error("too many columns");
        if (node.rows.length > WIDGET_LIMITS.tableRows) throw new Error("too many rows");
        for (const row of node.rows) {
          if (row.length !== node.columns.length) throw new Error("ragged row");
        }
        cells += node.rows.length * node.columns.length;
        break;
      case "kv":
        items += node.items.length;
        break;
      case "list":
        items += node.items.length;
        break;
      case "steps":
        items += node.steps.length;
        break;
      case "choice":
        items += node.options.length;
        for (const option of node.options) {
          if (!WIDGET_ID_PATTERN.test(option.value))
            throw new Error(`prose option value ${option.value}`);
        }
        break;
      case "image":
        images += 1;
        break;
      case "gallery":
        images += node.images.length;
        break;
      case "card":
        if ((node.actions?.length ?? 0) > WIDGET_LIMITS.cardActions)
          throw new Error("too many card actions");
        break;
      default:
        break;
    }
    for (const child of children(node)) visit(child, depth + 1);
  };
  visit(document.root, 1);
  if (nodes > WIDGET_LIMITS.nodes) throw new Error("too many nodes");
  if (deepest > WIDGET_LIMITS.nodeDepth) throw new Error("too deep");
  if (cells > WIDGET_LIMITS.tableCellsPerDocument) throw new Error("too many cells");
  if (items > WIDGET_LIMITS.itemsPerDocument) throw new Error("too many items");
  if (images > WIDGET_LIMITS.imagesPerDocument) throw new Error("too many images");
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
  // an agent writes today in the same language. This only guards against the
  // contract drifting into a shape that dwarfs the prose it replaces; the
  // measured ratios live in the design document.
  test(`widget example does not dwarf its Markdown: ${name}`, () => {
    const ratio = bytes(document) / bytes(markdown);
    if (ratio > 4) throw new Error(`${name}: widget is ${ratio.toFixed(2)}× the Markdown`);
  });
}

test("a patch costs what it changes, not what it repeats", () => {
  const patch = bytes(progressPatch);
  if (patch > WIDGET_LIMITS.documentBytes / 8) throw new Error("patch example too large");
  if (progressPatch.length > WIDGET_LIMITS.patchOps) throw new Error("too many ops");
  // The equivalent re-sent status line is a floor, not a ceiling: the patch
  // must stay below re-sending the approval card it updates.
  if (patch >= bytes(approval)) throw new Error("patch is not cheaper than a re-send");
  if (bytes(progressMarkdown) === 0) throw new Error("missing baseline");
});

test("the composed tap message carries only what the person saw", () => {
  const [first, second, ...rest] = approvalTapMessage.split("\n");
  if (rest.length !== 0 || first === undefined || second === undefined)
    throw new Error("two lines");
  const label = first.replace(/^\[widget [A-Za-z0-9_-]+ r\d+\] /, "");
  const visible = approval.root.type === "card" ? approval.root.actions?.map((b) => b.label) : [];
  if (!visible?.includes(label)) throw new Error("first line is not a visible label");
  const payload = JSON.parse(second) as Record<string, unknown>;
  for (const key of Object.keys(payload)) {
    if (!["widget", "revision", "node", "values", "labels"].includes(key))
      throw new Error(`hidden field ${key}`);
  }
});
