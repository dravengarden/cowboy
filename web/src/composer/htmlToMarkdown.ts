// Rich-text paste conversion, matching Obsidian's "Auto convert HTML" (on by
// default): copied web pages, documents and chats paste as Markdown instead of
// flattened plain text. Works on a DOM-like tree so the browser's own HTML
// parser handles malformed markup; the result is text only and never renders
// the pasted HTML.

export interface HtmlNodeLike {
  readonly nodeType: number;
  readonly nodeName: string;
  readonly textContent: string | null;
  readonly childNodes: ArrayLike<HtmlNodeLike>;
  getAttribute?(name: string): string | null;
}

const TEXT = 3;
const ELEMENT = 1;
const SKIP = new Set([
  "SCRIPT",
  "STYLE",
  "TEMPLATE",
  "HEAD",
  "META",
  "LINK",
  "TITLE",
  "NOSCRIPT",
]);
const BLOCK = new Set([
  "P",
  "DIV",
  "SECTION",
  "ARTICLE",
  "HEADER",
  "FOOTER",
  "MAIN",
  "ASIDE",
  "NAV",
  "FIGURE",
  "FIGCAPTION",
  "DL",
  "DT",
  "DD",
  "ADDRESS",
]);
/** Tags that carry Markdown meaning. Without one, the plain-text flavor is
 * already the best representation (e.g. code editors' styled spans). */
const SEMANTIC =
  /<(h[1-6]|strong|b|em|i|a\s[^>]*href|ul|ol|li|blockquote|pre|code|table|img|del|s|strike|mark|hr)\b/i;

export function htmlHasMarkdownStructure(html: string): boolean {
  return SEMANTIC.test(html);
}

function children(node: HtmlNodeLike): HtmlNodeLike[] {
  return Array.from(node.childNodes);
}

function attr(node: HtmlNodeLike, name: string): string {
  return node.getAttribute?.(name)?.trim() ?? "";
}

function safeUrl(url: string): string | null {
  if (!/^(https?:|mailto:)/i.test(url)) return null;
  return url.replace(/[()\s]/g, (c) => encodeURIComponent(c));
}

/** Escape characters that would turn pasted prose into formatting. An
 * underscore inside a word (snake_case) is literal in Markdown already. */
function escapeInline(text: string): string {
  return text.replace(/[\\`*[\]]/g, (c) => `\\${c}`)
    .replace(/(^|\W)_|_(?=\W|$)/g, (m) => m.replace("_", "\\_"));
}

function wrap(marker: string, inner: string): string {
  const trimmed = inner.trim();
  if (!trimmed) return inner;
  const lead = inner.slice(0, inner.length - inner.trimStart().length);
  const tail = inner.slice(inner.trimEnd().length);
  return `${lead}${marker}${trimmed}${marker}${tail}`;
}

interface Context {
  readonly listDepth: number;
  readonly pre: boolean;
}

function inline(node: HtmlNodeLike, ctx: Context): string {
  if (node.nodeType === TEXT) {
    const value = node.textContent ?? "";
    return ctx.pre ? value : escapeInline(value.replace(/\s+/g, " "));
  }
  if (node.nodeType !== ELEMENT || SKIP.has(node.nodeName)) return "";
  const inner = (): string =>
    children(node).map((child) => inline(child, ctx)).join("");
  switch (node.nodeName) {
    case "BR":
      return "\n";
    case "STRONG":
    case "B":
      return wrap("**", inner());
    case "EM":
    case "I":
      return wrap("*", inner());
    case "DEL":
    case "S":
    case "STRIKE":
      return wrap("~~", inner());
    case "MARK":
      return wrap("==", inner());
    case "CODE": {
      const text = node.textContent ?? "";
      if (ctx.pre) return text;
      const fence = text.includes("`") ? "``" : "`";
      return text ? `${fence}${text}${fence}` : "";
    }
    case "A": {
      const text = inner().trim();
      const href = safeUrl(attr(node, "href"));
      if (!href) return text;
      return text && text !== href ? `[${text}](${href})` : href;
    }
    case "IMG": {
      const src = safeUrl(attr(node, "src"));
      const alt = escapeInline(attr(node, "alt"));
      // Data URIs and blobs are not portable Markdown; keep their text.
      return src ? `![${alt}](${src})` : alt;
    }
    default:
      return BLOCK.has(node.nodeName) || isBlock(node)
        ? `\n${blocks(node, ctx)}\n`
        : inner();
  }
}

function isBlock(node: HtmlNodeLike): boolean {
  return /^(H[1-6]|UL|OL|LI|BLOCKQUOTE|PRE|TABLE|HR)$/.test(node.nodeName) ||
    BLOCK.has(node.nodeName);
}

function prefixLines(text: string, first: string, rest: string): string {
  return text.split("\n").map((line, i) =>
    (i === 0 ? first : line ? rest : rest.trimEnd()) + line
  )
    .join("\n");
}

function table(node: HtmlNodeLike, ctx: Context): string {
  const rows: string[][] = [];
  const visit = (n: HtmlNodeLike): void => {
    for (const child of children(n)) {
      if (child.nodeName === "TR") {
        rows.push(
          children(child)
            .filter((c) => c.nodeName === "TD" || c.nodeName === "TH")
            .map((c) =>
              inline(c, ctx).replace(/\s*\n\s*/g, " ").replace(/\|/g, "\\|")
                .trim()
            ),
        );
      } else if (child.nodeType === ELEMENT) visit(child);
    }
  };
  visit(node);
  const width = Math.max(0, ...rows.map((r) => r.length));
  if (width === 0) return "";
  const line = (cells: string[]): string =>
    `| ${
      Array.from({ length: width }, (_, i) => cells[i] ?? "").join(" | ")
    } |`;
  return [
    line(rows[0]!),
    `| ${Array(width).fill("---").join(" | ")} |`,
    ...rows.slice(1).map(line),
  ]
    .join("\n");
}

function block(node: HtmlNodeLike, ctx: Context): string {
  switch (node.nodeName) {
    case "H1":
    case "H2":
    case "H3":
    case "H4":
    case "H5":
    case "H6":
      return `${"#".repeat(Number(node.nodeName[1]))} ${
        inline(node, ctx).replace(/\s+/g, " ").trim()
      }`;
    case "HR":
      return "---";
    case "PRE": {
      const code = (node.textContent ?? "").replace(/\n$/, "");
      const languageNode = children(node).find((c) => c.nodeName === "CODE");
      const language = /language-([\w+#-]+)/.exec(
        languageNode ? attr(languageNode, "class") : attr(node, "class"),
      )?.[1] ?? "";
      const fence = code.includes("```") ? "````" : "```";
      return `${fence}${language}\n${code}\n${fence}`;
    }
    case "BLOCKQUOTE":
      return prefixLines(blocks(node, ctx), "> ", "> ");
    case "UL":
    case "OL": {
      let index = Number(attr(node, "start")) || 1;
      const nested: Context = { ...ctx, listDepth: ctx.listDepth + 1 };
      return children(node).filter((c) => c.nodeName === "LI").map((item) => {
        const checkbox = children(item).find((c) =>
          c.nodeName === "INPUT" && attr(c, "type").toLowerCase() === "checkbox"
        );
        const marker = node.nodeName === "OL" ? `${index++}. ` : "- ";
        const task = checkbox
          ? (checkbox.getAttribute?.("checked") !== null ? "[x] " : "[ ] ")
          : "";
        const body = blocks(item, nested).replace(/^\n+|\n+$/g, "").replace(
          /\n{2,}/g,
          "\n",
        );
        return prefixLines(body, marker + task, " ".repeat(marker.length));
      }).join("\n");
    }
    case "TABLE":
      return table(node, ctx);
    default:
      return blocks(node, ctx);
  }
}

/** Convert a node's children, separating block elements by blank lines and
 * keeping runs of inline content together. */
function blocks(node: HtmlNodeLike, ctx: Context): string {
  const parts: string[] = [];
  let run = "";
  const flush = (): void => {
    const text = run.replace(/[ \t]+\n/g, "\n").replace(/\n[ \t]+/g, "\n")
      .trim();
    if (text) parts.push(text);
    run = "";
  };
  for (const child of children(node)) {
    if (
      child.nodeType === ELEMENT && !SKIP.has(child.nodeName) && isBlock(child)
    ) {
      flush();
      const text = block(child, ctx).trim();
      if (text) parts.push(text);
    } else {
      run += inline(child, ctx);
    }
  }
  flush();
  return parts.join(ctx.listDepth > 0 ? "\n" : "\n\n");
}

export function htmlNodeToMarkdown(root: HtmlNodeLike): string {
  return blocks(root, { listDepth: 0, pre: false }).replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** Markdown for a clipboard HTML payload, or null when the plain-text flavor
 * should be pasted instead (no semantic markup, or nothing convertible). */
export function clipboardHtmlToMarkdown(html: string): string | null {
  if (!htmlHasMarkdownStructure(html) || typeof DOMParser === "undefined") {
    return null;
  }
  try {
    const doc = new DOMParser().parseFromString(html, "text/html");
    const markdown = htmlNodeToMarkdown(doc.body as unknown as HtmlNodeLike);
    return markdown === "" ? null : markdown;
  } catch {
    return null;
  }
}
