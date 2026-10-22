// react-syntax-highlighter's inline-style renderer with its per-token work
// cached.
//
// The library's default renderer rebuilds the theme's whole selector list for
// every token it renders, which made opening a tool-heavy transcript spend
// most of its time in the highlighter. This renderer produces the same
// elements, class names and merged inline styles (same precedence, including
// nested token classes such as `tag attr-name`), but computes each distinct
// class list once per theme.
import { createElement, type CSSProperties, type ReactNode } from "react";
import * as rshElement from "react-syntax-highlighter/dist/esm/create-element";

type Stylesheet = Readonly<Record<string, CSSProperties>>;
interface HastNode {
  type: "element" | "text";
  value?: string | number | undefined;
  tagName?: unknown;
  properties?: {
    className?: string[];
    style?: CSSProperties;
    [key: string]: unknown;
  };
  children?: HastNode[];
}

const createStyleObject = (rshElement as unknown as {
  createStyleObject: (
    classNames: string[],
    elementStyle: CSSProperties,
    stylesheet: Stylesheet,
  ) => CSSProperties;
}).createStyleObject;

interface ThemeCache {
  selectors: ReadonlySet<string>;
  byClasses: Map<
    string,
    { className: string | undefined; style: CSSProperties }
  >;
}
const caches = new WeakMap<Stylesheet, ThemeCache>();

function themeCache(stylesheet: Stylesheet): ThemeCache {
  let cache = caches.get(stylesheet);
  if (!cache) {
    const selectors = new Set<string>();
    for (const selector of Object.keys(stylesheet)) {
      for (const part of selector.split(".")) selectors.add(part);
    }
    cache = { selectors, byClasses: new Map() };
    caches.set(stylesheet, cache);
  }
  return cache;
}

/** The class name and inline style the library's renderer gives `classNames`. */
export function inlineTokenProps(
  classNames: readonly string[],
  stylesheet: Stylesheet,
): { className: string | undefined; style: CSSProperties } {
  const cache = themeCache(stylesheet);
  const key = classNames.join(" ");
  let props = cache.byClasses.get(key);
  if (!props) {
    // Keep `token` for compatibility, plus every class the theme does not style.
    const kept = [
      ...(classNames.includes("token") ? ["token"] : []),
      ...classNames.filter((name) => !cache.selectors.has(name)),
    ];
    props = {
      className: kept.join(" ") || undefined,
      style: createStyleObject([...classNames], {}, stylesheet),
    };
    cache.byClasses.set(key, props);
  }
  return props;
}

function renderNode(
  node: HastNode,
  stylesheet: Stylesheet,
  key: string,
): ReactNode {
  if (node.type === "text") return node.value;
  if (!node.tagName) return null;
  const { className = [], style, ...properties } = node.properties ?? {};
  const token = inlineTokenProps(className, stylesheet);
  // A row's own style (wrapLines lineProps) is the base the theme merges over.
  const merged = style ? { ...style, ...token.style } : token.style;
  return createElement(
    node.tagName as string,
    { ...properties, key, className: token.className, style: merged },
    ...(node.children ?? []).map((child, index) =>
      renderNode(child, stylesheet, `${key}-${String(index)}`)
    ),
  );
}

/** Pass as `renderer` with `useInlineStyles` (the default). */
export function cachedInlineRenderer(
  { rows, stylesheet }: { rows: HastNode[]; stylesheet: Stylesheet },
): ReactNode {
  return rows.map((row, index) =>
    renderNode(row, stylesheet, `code-segment-${String(index)}`)
  );
}
