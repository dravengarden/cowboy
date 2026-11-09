// Parses and validates a `cowboy.widget/1` document for display
// (docs/widgets.md §5.3). The Web is tolerant only of node types it does not
// know: they become explicit `unsupported` nodes so the closed union stays
// honest. Any other shape or bound violation rejects the whole document and
// the caller shows the source instead.

import { strictJson } from "../../../contracts/strict-json.ts";
import {
  LABEL_PATTERN,
  type Node,
  WIDGET_ID_PATTERN,
  WIDGET_LIMITS,
  WIDGET_SCHEMA,
  type WidgetDocument,
} from "../../../contracts/widget-v1.ts";

export type UnsupportedNode = {
  type: "unsupported";
  original: string;
  fallback?: string;
  id?: string;
};

type Containers = Extract<Node, { children: Node[] }>;
type WithParsedChildren<T> = Omit<T, "children"> & { children: ParsedNode[] };
export type ParsedNode =
  | Exclude<Node, Containers>
  | WithParsedChildren<Extract<Node, { type: "stack" }>>
  | WithParsedChildren<Extract<Node, { type: "section" }>>
  | WithParsedChildren<Extract<Node, { type: "card" }>>
  | UnsupportedNode;

export type ParsedDocument = Omit<WidgetDocument, "root"> & { root: ParsedNode };

export type ParseResult = { ok: true; document: ParsedDocument } | { ok: false; error: string };

const KNOWN = new Set([
  "stack",
  "card",
  "section",
  "divider",
  "text",
  "kv",
  "list",
  "table",
  "link",
  "file",
  "status",
  "progress",
  "steps",
  "alert",
  "image",
  "gallery",
  "button",
  "choice",
  "app",
]);
const TONES = new Set(["neutral", "info", "success", "warning", "error", "running"]);
const STEP_STATES = new Set(["pending", "active", "done", "failed", "skipped"]);
const SEVERITIES = new Set(["info", "success", "warning", "error"]);

class Invalid extends Error {}

const chars = (value: string): number => Array.from(value).length;
const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

function fail(path: string, what: string): never {
  throw new Invalid(`${path}: ${what}`);
}

function text(value: unknown, path: string, max: number, optional = false): void {
  if (value === undefined && optional) return;
  if (typeof value !== "string") fail(path, "expected text");
  if (chars(value) > max) fail(path, "too long");
}

function line(value: unknown, path: string, max: number, optional = false): void {
  text(value, path, max, optional);
  if (typeof value === "string" && (!LABEL_PATTERN.test(value) || !value.trim())) {
    fail(path, "must be one printable line");
  }
}

function array(value: unknown, path: string, max: number): unknown[] {
  if (!Array.isArray(value)) fail(path, "expected a list");
  if (value.length > max) fail(path, "too many entries");
  return value;
}

function tone(value: unknown, path: string): void {
  if (value !== undefined && (typeof value !== "string" || !TONES.has(value)))
    fail(path, "unknown tone");
}

interface Budget {
  nodes: number;
  cells: number;
  items: number;
  images: number;
  ids: Set<string>;
}

function cellValue(value: unknown, path: string): void {
  if (value === null || typeof value === "boolean" || typeof value === "number") return;
  if (typeof value === "string") return text(value, path, WIDGET_LIMITS.labelChars * 4);
  if (!isObject(value)) fail(path, "expected a cell");
  const v = value["v"];
  if (!(v === null || ["string", "number", "boolean"].includes(typeof v)))
    fail(`${path}.v`, "expected a value");
  if (typeof v === "string") text(v, `${path}.v`, WIDGET_LIMITS.labelChars * 4);
  tone(value["tone"], `${path}.tone`);
}

function listItems(value: unknown, path: string, depth: number): number {
  if (depth > WIDGET_LIMITS.nodeDepth) fail(path, "nested too deeply");
  let count = 0;
  array(value, path, WIDGET_LIMITS.listItems).forEach((item, index) => {
    const at = `${path}[${index}]`;
    if (!isObject(item)) fail(at, "expected an item");
    text(item["text"], `${at}.text`, WIDGET_LIMITS.textChars);
    tone(item["tone"], `${at}.tone`);
    count += 1;
    if (item["children"] !== undefined)
      count += listItems(item["children"], `${at}.children`, depth + 1);
  });
  return count;
}

/** An agent-written app (docs/widget-apps.md §8): bounds only; `props`
 *  stays opaque and every image is a source the Controller verified. */
function app(node: Record<string, unknown>, path: string, budget: Budget): void {
    if (typeof node["id"] !== "string") fail(`${path}.id`, "required");
    line(node["title"], `${path}.title`, WIDGET_LIMITS.titleChars);
    text(node["fallback"], `${path}.fallback`, 2048);
    if (
      typeof node["code"] !== "string" ||
      JSON.stringify(node["code"]).length > WIDGET_LIMITS.appCodeBytes
    ) {
      fail(`${path}.code`, `expected TSX of at most ${WIDGET_LIMITS.appCodeBytes} bytes`);
    }
    if (node["props"] !== undefined) {
      if (
        !isObject(node["props"]) ||
        JSON.stringify(node["props"]).length > WIDGET_LIMITS.appPropsBytes
      ) {
        fail(
          `${path}.props`,
          `expected an object of at most ${WIDGET_LIMITS.appPropsBytes} bytes`,
        );
      }
    }
    if (node["images"] !== undefined) {
      if (!isObject(node["images"])) fail(`${path}.images`, "expected an object");
      const entries = Object.entries(node["images"] as Record<string, unknown>);
      if (entries.length > WIDGET_LIMITS.appImages) fail(`${path}.images`, "at most 8 images");
      for (const [key, source] of entries) {
        if (!/^[A-Za-z0-9_-]{1,64}$/.test(key)) fail(`${path}.images.${key}`, "expected an id");
        if (!isObject(source)) fail(`${path}.images.${key}`, "expected an image source");
      }
      budget.images += entries.length;
    }
}

function leaf(node: Record<string, unknown>, type: string, path: string, budget: Budget): void {
  switch (type) {
    case "divider":
      return;
    case "text":
      text(node["text"], `${path}.text`, WIDGET_LIMITS.textChars);
      return tone(node["tone"], `${path}.tone`);
    case "kv": {
      const items = array(node["items"], `${path}.items`, WIDGET_LIMITS.kvItems);
      items.forEach((item, index) => {
        if (!isObject(item)) fail(`${path}.items[${index}]`, "expected an item");
        text(item["k"], `${path}.items[${index}].k`, WIDGET_LIMITS.labelChars);
        cellValue(item["v"], `${path}.items[${index}].v`);
      });
      budget.items += items.length;
      return;
    }
    case "list": {
      const count = listItems(node["items"], `${path}.items`, 1);
      if (count > WIDGET_LIMITS.listItems) fail(`${path}.items`, "too many entries");
      budget.items += count;
      return;
    }
    case "table": {
      const columns = array(node["columns"], `${path}.columns`, WIDGET_LIMITS.tableColumns);
      columns.forEach((column, index) => {
        if (typeof column === "string")
          return text(column, `${path}.columns[${index}]`, WIDGET_LIMITS.labelChars);
        if (!isObject(column)) fail(`${path}.columns[${index}]`, "expected a column");
        text(column["label"], `${path}.columns[${index}].label`, WIDGET_LIMITS.labelChars);
      });
      const rows = array(node["rows"], `${path}.rows`, WIDGET_LIMITS.tableRows);
      rows.forEach((row, r) => {
        const cells = array(row, `${path}.rows[${r}]`, WIDGET_LIMITS.tableColumns);
        if (cells.length !== columns.length)
          fail(`${path}.rows[${r}]`, "row width differs from columns");
        cells.forEach((cell, c) => cellValue(cell, `${path}.rows[${r}][${c}]`));
      });
      budget.cells += rows.length * columns.length;
      return;
    }
    case "link":
      line(node["label"], `${path}.label`, WIDGET_LIMITS.labelChars);
      if (!isObject(node["to"])) fail(`${path}.to`, "expected a reference");
      return;
    case "file":
      return text(node["path"], `${path}.path`, 1024);
    case "status":
      line(node["label"], `${path}.label`, WIDGET_LIMITS.labelChars);
      if (!TONES.has(String(node["tone"]))) fail(`${path}.tone`, "unknown tone");
      return;
    case "progress": {
      const value = node["value"];
      if (value !== undefined && (typeof value !== "number" || value < 0 || value > 1))
        fail(`${path}.value`, "expected 0..1");
      return tone(node["tone"], `${path}.tone`);
    }
    case "steps": {
      const steps = array(node["steps"], `${path}.steps`, WIDGET_LIMITS.steps);
      steps.forEach((step, index) => {
        if (!isObject(step)) fail(`${path}.steps[${index}]`, "expected a step");
        text(step["label"], `${path}.steps[${index}].label`, WIDGET_LIMITS.labelChars);
        if (!STEP_STATES.has(String(step["state"])))
          fail(`${path}.steps[${index}].state`, "unknown state");
      });
      budget.items += steps.length;
      return;
    }
    case "alert":
      if (!SEVERITIES.has(String(node["severity"]))) fail(`${path}.severity`, "unknown severity");
      return text(node["text"], `${path}.text`, WIDGET_LIMITS.textChars);
    case "image":
      text(node["alt"], `${path}.alt`, WIDGET_LIMITS.labelChars);
      if (!isObject(node["src"])) fail(`${path}.src`, "expected an image source");
      budget.images += 1;
      return;
    case "gallery": {
      const images = array(node["images"], `${path}.images`, WIDGET_LIMITS.galleryImages);
      images.forEach((image, index) => {
        if (!isObject(image)) fail(`${path}.images[${index}]`, "expected an image");
        text(image["alt"], `${path}.images[${index}].alt`, WIDGET_LIMITS.labelChars);
      });
      budget.images += images.length;
      return;
    }
    case "app":
      app(node, path, budget);
      return;
    case "button":
      if (typeof node["id"] !== "string") fail(`${path}.id`, "required");
      line(node["label"], `${path}.label`, WIDGET_LIMITS.labelChars);
      if (!isObject(node["action"])) fail(`${path}.action`, "required");
      {
        // Every action field a renderer reads is checked here, so a malformed
        // published document falls back instead of breaking the transcript.
        const action = node["action"] as Record<string, unknown>;
        const kind = action["kind"];
        if (kind !== "prompt" && kind !== "copy" && kind !== "open") {
          fail(`${path}.action.kind`, "expected prompt, copy or open");
        }
        if (kind === "copy") text(action["text"], `${path}.action.text`, 8 * 1024);
        if (kind === "open") {
          const to = action["to"];
          if (!isObject(to) || typeof to["kind"] !== "string")
            fail(`${path}.action.to`, "required");
          if (to["kind"] === "url" && typeof to["url"] !== "string") {
            fail(`${path}.action.to.url`, "expected text");
          }
        }
      }
      return;
    case "choice": {
      if (typeof node["id"] !== "string") fail(`${path}.id`, "required");
      line(node["label"], `${path}.label`, WIDGET_LIMITS.labelChars);
      const options = array(node["options"], `${path}.options`, WIDGET_LIMITS.options);
      const values = new Set<string>();
      options.forEach((option, index) => {
        if (!isObject(option)) fail(`${path}.options[${index}]`, "expected an option");
        const value = option["value"];
        if (typeof value !== "string" || !WIDGET_ID_PATTERN.test(value) || values.has(value)) {
          fail(`${path}.options[${index}].value`, "expected a unique identifier");
        }
        values.add(value);
        line(option["label"], `${path}.options[${index}].label`, WIDGET_LIMITS.labelChars);
      });
      budget.items += options.length;
      return;
    }
  }
}

function parseNode(value: unknown, path: string, depth: number, budget: Budget): ParsedNode {
  if (!isObject(value)) fail(path, "expected a node");
  if (depth > WIDGET_LIMITS.nodeDepth) fail(path, "nested too deeply");
  budget.nodes += 1;
  if (budget.nodes > WIDGET_LIMITS.nodes) fail(path, "too many nodes");
  const id = value["id"];
  if (id !== undefined) {
    if (typeof id !== "string" || !WIDGET_ID_PATTERN.test(id) || budget.ids.has(id))
      fail(`${path}.id`, "invalid or duplicate id");
    budget.ids.add(id);
  }
  text(value["fallback"], `${path}.fallback`, WIDGET_LIMITS.fallbackChars, true);
  const type = value["type"];
  if (typeof type !== "string") fail(`${path}.type`, "required");
  if (!KNOWN.has(type)) {
    const unsupported: UnsupportedNode = { type: "unsupported", original: type };
    if (typeof value["fallback"] === "string") unsupported.fallback = value["fallback"];
    return unsupported;
  }
  if (type === "stack" || type === "card" || type === "section") {
    if (type === "section") line(value["title"], `${path}.title`, WIDGET_LIMITS.titleChars);
    if (type === "card") {
      line(value["title"], `${path}.title`, WIDGET_LIMITS.titleChars, true);
      const actions =
        value["actions"] === undefined
          ? []
          : array(value["actions"], `${path}.actions`, WIDGET_LIMITS.cardActions);
      actions.forEach((action, index) => {
        const parsed = parseNode(action, `${path}.actions[${index}]`, depth + 1, budget);
        if (parsed.type !== "button")
          fail(`${path}.actions[${index}]`, "card actions must be buttons");
      });
    }
    const children = array(value["children"], `${path}.children`, WIDGET_LIMITS.nodes).map(
      (child, index) => parseNode(child, `${path}.children[${index}]`, depth + 1, budget),
    );
    return { ...(value as unknown as Containers), children } as ParsedNode;
  }
  leaf(value, type, path, budget);
  return value as unknown as ParsedNode;
}

/** Validates an already parsed document (a published widget snapshot). */
export function parseWidgetValue(value: unknown): ParseResult {
  try {
    if (!isObject(value)) fail("document", "expected an object");
    if (value["schema"] !== WIDGET_SCHEMA) fail("schema", `expected ${WIDGET_SCHEMA}`);
    line(value["title"], "title", WIDGET_LIMITS.titleChars);
    line(value["summary"], "summary", WIDGET_LIMITS.summaryChars, true);
    const budget: Budget = { nodes: 0, cells: 0, items: 0, images: 0, ids: new Set() };
    const root = parseNode(value["root"], "root", 1, budget);
    if (budget.cells > WIDGET_LIMITS.tableCellsPerDocument)
      fail("document", "too many table cells");
    if (budget.items > WIDGET_LIMITS.itemsPerDocument) fail("document", "too many items");
    if (budget.images > WIDGET_LIMITS.imagesPerDocument) fail("document", "too many images");
    return { ok: true, document: { ...(value as unknown as WidgetDocument), root } };
  } catch (error) {
    if (error instanceof Invalid) return { ok: false, error: error.message };
    throw error;
  }
}

/** Parses the body of a ```cowboy-widget fence. */
export function parseWidgetSource(raw: string): ParseResult {
  let value: unknown;
  try {
    value = strictJson(raw, WIDGET_LIMITS.documentBytes, WIDGET_LIMITS.jsonDepth);
  } catch {
    return { ok: false, error: "not valid JSON (or over the size or depth bound)" };
  }
  return parseWidgetValue(value);
}
