// The host's copy of an app's component tree (docs/widget-apps.md §5.2, §6).
// Render batches arrive from the trusted Worker already shape-checked; this
// model re-validates them, applies them and enforces the expansion budget
// before anything reaches React. A violation stops the app.

import { ELEMENT_NAMES } from "./runtime/limits.js";

export type AppProps = Readonly<Record<string, unknown>> & { __handlers?: readonly string[] };

export type AppElement = {
  readonly id: number;
  readonly type: string;
  readonly props: AppProps;
  readonly children: readonly number[];
};

export type AppText = { readonly id: number; readonly text: string };

export type AppTreeState = {
  readonly nodes: ReadonlyMap<number, AppElement | AppText>;
  readonly root: readonly number[];
  /** Expansion cost of the whole tree (§6). */
  readonly units: number;
};

export const APP_BUDGET = {
  units: 6000,
  retainedBytes: 512 * 1024,
  textChars: 40_000,
  chartPoints: 2000,
} as const;

const elements = new Set(ELEMENT_NAMES);

export function emptyAppTree(): AppTreeState {
  return { nodes: new Map(), root: [], units: 0 };
}

export function isText(node: AppElement | AppText | undefined): node is AppText {
  return node !== undefined && "text" in node;
}

/** Units one node costs: 1, plus cells, items, options and points it expands to. */
export function nodeUnits(node: AppElement | AppText): number {
  if (isText(node)) return 1 + Math.floor(node.text.length / 200);
  const p = node.props;
  const length = (value: unknown): number => (Array.isArray(value) ? value.length : 0);
  switch (node.type) {
    case "Table": {
      const rows = Array.isArray(p["rows"]) ? (p["rows"] as unknown[]) : [];
      return 1 + length(p["columns"]) + rows.reduce<number>((sum, row) => sum + length(row), 0);
    }
    case "List":
      return 1 + length(p["items"]);
    case "Select":
    case "Tabs":
      return 1 + length(p["options"]);
    case "Chart": {
      const series = Array.isArray(p["series"]) ? (p["series"] as unknown[]).length : 1;
      return 1 + length(p["data"]) * Math.max(1, series);
    }
    default:
      return 1;
  }
}

export class AppBudgetError extends Error {}

/** Characters of text a node's props carry (labels, cells, items). */
function textOf(value: unknown): number {
  if (typeof value === "string") return value.length;
  if (Array.isArray(value)) return value.reduce<number>((sum, item) => sum + textOf(item), 0);
  if (typeof value === "object" && value !== null) {
    return Object.values(value).reduce<number>((sum, item) => sum + textOf(item), 0);
  }
  return 0;
}

type Op =
  | ["create", number, string, AppProps]
  | ["text", number, string]
  | ["props", number, AppProps]
  | ["append", number, number]
  | ["before", number, number, number]
  | ["remove", number]
  | ["clear"];

function children(
  state: Map<number, AppElement | AppText>,
  root: number[],
  parent: number,
): number[] {
  if (parent === 0) return root;
  const node = state.get(parent);
  if (!node || isText(node)) throw new AppBudgetError("render update addressed an unknown node");
  return node.children as number[];
}

function detach(nodes: Map<number, AppElement | AppText>, root: number[], id: number): void {
  const index = root.indexOf(id);
  if (index >= 0) root.splice(index, 1);
  for (const node of nodes.values()) {
    if (!isText(node)) {
      const at = node.children.indexOf(id);
      if (at >= 0) (node.children as number[]).splice(at, 1);
    }
  }
}

function drop(nodes: Map<number, AppElement | AppText>, id: number): void {
  const node = nodes.get(id);
  nodes.delete(id);
  if (node && !isText(node)) for (const child of node.children) drop(nodes, child);
}

/** Applies one batch (JSON text). Returns the new state or throws AppBudgetError. */
export function applyAppOps(state: AppTreeState, json: string): AppTreeState {
  let ops: unknown;
  try {
    ops = JSON.parse(json);
  } catch {
    throw new AppBudgetError("render update was not valid JSON");
  }
  if (!Array.isArray(ops)) throw new AppBudgetError("render update was not a list");
  const nodes = new Map<number, AppElement | AppText>();
  for (const [id, node] of state.nodes) {
    nodes.set(id, isText(node) ? node : { ...node, children: [...node.children] });
  }
  const root = [...state.root];
  for (const raw of ops as Op[]) {
    switch (raw[0]) {
      case "create":
        if (!elements.has(raw[2])) throw new AppBudgetError(`unknown component ${raw[2]}`);
        nodes.set(raw[1], { id: raw[1], type: raw[2], props: raw[3] ?? {}, children: [] });
        break;
      case "text":
        nodes.set(raw[1], { id: raw[1], text: String(raw[2]) });
        break;
      case "props": {
        const node = nodes.get(raw[1]);
        if (!node || isText(node)) throw new AppBudgetError("props for an unknown node");
        nodes.set(raw[1], { ...node, props: raw[2] ?? {} });
        break;
      }
      case "append":
        detach(nodes, root, raw[2]);
        children(nodes, root, raw[1]).push(raw[2]);
        break;
      case "before": {
        detach(nodes, root, raw[2]);
        const list = children(nodes, root, raw[1]);
        const at = list.indexOf(raw[3]);
        list.splice(at < 0 ? list.length : at, 0, raw[2]);
        break;
      }
      case "remove":
        detach(nodes, root, raw[1]);
        drop(nodes, raw[1]);
        break;
      case "clear":
        // React clears the container on first mount after creating the new
        // instances: only the container's children go, not the nodes.
        for (const id of root.splice(0)) drop(nodes, id);
        break;
      default:
        throw new AppBudgetError("malformed render update");
    }
  }
  let units = 0;
  let bytes = 0;
  let chars = 0;
  let points = 0;
  const encoder = new TextEncoder();
  for (const node of nodes.values()) {
    units += nodeUnits(node);
    bytes += encoder.encode(isText(node) ? node.text : JSON.stringify(node.props)).length;
    chars += isText(node) ? node.text.length : textOf(node.props);
    if (!isText(node) && node.type === "Chart" && Array.isArray(node.props["data"])) {
      const series = Array.isArray(node.props["series"]) ? node.props["series"].length : 1;
      points += node.props["data"].length * Math.max(1, series);
    }
  }
  if (units > APP_BUDGET.units) throw new AppBudgetError("the app drew too much at once");
  if (bytes > APP_BUDGET.retainedBytes)
    throw new AppBudgetError("the app kept too much data on screen");
  if (chars > APP_BUDGET.textChars) throw new AppBudgetError("the app showed too much text");
  if (points > APP_BUDGET.chartPoints)
    throw new AppBudgetError("the app's charts had too many points");
  return { nodes, root, units };
}
