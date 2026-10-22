// A react-syntax-highlighter Prism theme as one scoped stylesheet.
//
// With `useInlineStyles` (the library default) every highlighted token
// recomputes the theme's whole selector list to decide its inline style, which
// made opening a tool-heavy transcript spend most of its time in the
// highlighter. Rendering class names and injecting the same theme once as CSS
// keeps the colours and removes that per-token work.

type ThemeStyle = object;
export type PrismTheme = Readonly<Record<string, ThemeStyle>>;

const UNITLESS = new Set([
  "opacity",
  "zIndex",
  "fontWeight",
  "lineHeight",
  "tabSize",
  "MozTabSize",
  "OTabSize",
  "flex",
]);

function declarations(style: ThemeStyle): string {
  return Object.entries(style as Record<string, unknown>)
    .filter(([, value]) => value !== undefined && value !== "")
    .map(([property, value]) => {
      const name = property.startsWith("--")
        ? property
        : property.replace(/^(Moz|O|Webkit|ms)(?=[A-Z])/, (prefix) =>
          `-${prefix.toLowerCase()}`)
          .replace(/[A-Z]/g, (letter) =>
            `-${letter.toLowerCase()}`);
      const text =
        typeof value === "number" && value !== 0 && !UNITLESS.has(property)
          ? `${value}px`
          : String(value);
      return `${name}:${text}`;
    })
    .join(";");
}

/** A Prism theme key names a selector (`pre[class*="language-"]`,
 * `.language-css .token.string`, `::selection`) or a token class
 * combination (`keyword`, `keyword.module`). Inline mode applies the latter
 * to any element carrying those classes; Prism marks each such element
 * `token`, so `.token.<classes>` is the equivalent selector. */
function selectorFor(key: string): string {
  return /[\s[:>*]/.test(key) || key.startsWith(".") ? key : `.token.${key}`;
}

export function prismThemeCss(theme: PrismTheme, scope: string): string {
  const rules: string[] = [];
  for (const [key, style] of Object.entries(theme)) {
    // Inline styles never reached pseudo-elements such as `::selection`;
    // keep the browser's selection exactly as before.
    if (key.includes("::")) continue;
    const body = declarations(style);
    if (!body) continue;
    const selectors = key.split(",").map((part) =>
      `.${scope} ${selectorFor(part.trim())}`
    ).join(",");
    rules.push(`${selectors}{${body}}`);
  }
  return rules.join("\n");
}

const installed = new Set<string>();

/** Install a theme's stylesheet once per document; returns its scope class. */
export function installPrismTheme(theme: PrismTheme, scope: string): string {
  if (!installed.has(scope) && typeof document !== "undefined") {
    installed.add(scope);
    const element = document.createElement("style");
    element.dataset.cowboyPrism = scope;
    element.textContent = prismThemeCss(theme, scope);
    document.head.append(element);
  }
  return scope;
}
