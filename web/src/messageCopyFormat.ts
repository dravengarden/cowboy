// Pure synchronous serializer for message Copy (clean semantic HTML + readable
// plain text) and Copy as Markdown (exact raw Markdown source).
//
// Zero runtime dependency on CodeMirror or react-markdown so copying never
// bloats the eager bundle, never leaks dark-mode theme styles or code-block UI
// chrome, never duplicates KaTeX formulas, and stays synchronous inside iOS
// Safari / WKWebView user gestures.

import {
  COMPACTING_NOTICE,
  type ContentChunk,
  isCompactionCommandText,
  isCompactionCompletionText,
  isHumanPrompt,
  type RenderItem,
} from "./derive";

export interface FormattedCopyPayload {
  readonly html: string;
  readonly plain: string;
}

export interface CopyableMessage {
  readonly markdown: string;
  readonly role: "assistant" | "user";
}

function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function safeHref(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol === "http:" || parsed.protocol === "https:" || parsed.protocol === "mailto:") {
      return parsed.href;
    }
  } catch {
    // Relative or malformed URL: do not emit broken href in external clipboard HTML.
  }
  return null;
}

function isFileHref(raw: string): boolean {
  return /^file:\/\//i.test(raw.trim());
}

/** Extract verbatim Markdown source from a message's content chunks. Drops
 *  non-text binary attachment previews so copied Markdown never leaks dead
 *  internal blob or artifact URLs. */
export function extractMessageMarkdown(chunks: readonly ContentChunk[]): string {
  const parts: string[] = [];
  for (const chunk of chunks) {
    if (chunk.type === "text" && chunk.text.length > 0) parts.push(chunk.text);
  }
  return parts.join("\n\n");
}

/** Identify whether a transcript RenderItem is a completed copyable message. */
export function extractCopyableMessage(item: RenderItem): CopyableMessage | null {
  if (item.kind !== "message") return null;
  const singleText =
    item.chunks.length === 1 && item.chunks[0]?.type === "text"
      ? item.chunks[0].text
      : null;
  if (item.role === "assistant") {
    if (singleText !== null && (singleText.trim() === COMPACTING_NOTICE || isCompactionCompletionText(singleText))) {
      return null;
    }
  } else {
    if (!isHumanPrompt(item.origin)) return null;
    if (singleText !== null && isCompactionCommandText(singleText)) return null;
  }
  const markdown = extractMessageMarkdown(item.chunks);
  if (!markdown.trim()) return null;
  return { markdown, role: item.role };
}

function findClosingCodeSpan(text: string, from: number, ticks: number): number {
  let i = from;
  while (i < text.length) {
    if (text[i] === "`") {
      let count = 0;
      while (i + count < text.length && text[i + count] === "`") count++;
      if (count === ticks) return i;
      i += count;
    } else {
      i++;
    }
  }
  return -1;
}

function parseLinkDestination(text: string, openParen: number): { url: string; end: number } | null {
  let i = openParen + 1;
  while (i < text.length && (text[i] === " " || text[i] === "\t")) i++;
  if (i >= text.length) return null;
  if (text[i] === "<") {
    const closeAngle = text.indexOf(">", i + 1);
    if (closeAngle < 0) return null;
    const url = text.slice(i + 1, closeAngle);
    let end = closeAngle + 1;
    while (end < text.length && text[end] !== ")") end++;
    return end < text.length ? { url, end: end + 1 } : null;
  }
  let depth = 0;
  const start = i;
  let urlEnd = -1;
  while (i < text.length) {
    const ch = text[i]!;
    if (ch === "\\" && i + 1 < text.length) {
      i += 2;
      continue;
    }
    if (ch === "(") {
      depth++;
    } else if (ch === ")") {
      if (depth === 0) {
        return { url: text.slice(start, urlEnd >= 0 ? urlEnd : i), end: i + 1 };
      }
      depth--;
    } else if ((ch === " " || ch === "\t") && depth === 0 && urlEnd < 0) {
      urlEnd = i;
    }
    i++;
  }
  return null;
}

function findMatchingBracket(text: string, open: number): number {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    const ch = text[i]!;
    if (ch === "\\" && i + 1 < text.length) {
      i++;
      continue;
    }
    if (ch === "`") {
      let ticks = 0;
      while (i + ticks < text.length && text[i + ticks] === "`") ticks++;
      const close = findClosingCodeSpan(text, i + ticks, ticks);
      if (close >= 0) {
        i = close + ticks - 1;
        continue;
      }
    }
    if (ch === "[") depth++;
    else if (ch === "]") {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

function isWordChar(ch: string | undefined): boolean {
  return ch !== undefined && /[\p{L}\p{N}_]/u.test(ch);
}

function findDelimitedClose(text: string, from: number, marker: string, requireWordBoundary: boolean): number {
  let i = from;
  while (i < text.length) {
    if (text[i] === "\\" && i + 1 < text.length) {
      i += 2;
      continue;
    }
    if (text[i] === "`") {
      let ticks = 0;
      while (i + ticks < text.length && text[i + ticks] === "`") ticks++;
      const close = findClosingCodeSpan(text, i + ticks, ticks);
      if (close >= 0) {
        i = close + ticks;
        continue;
      }
    }
    if (text.startsWith(marker, i)) {
      const prev = text[i - 1];
      const next = text[i + marker.length];
      if (prev !== " " && prev !== "\t" && (!requireWordBoundary || !isWordChar(next))) {
        return i;
      }
    }
    i++;
  }
  return -1;
}

function tryInlineLink(text: string, start: number, isImage: boolean): { html: string; plain: string; next: number } | null {
  const openBracket = isImage ? start + 1 : start;
  const closeBracket = findMatchingBracket(text, openBracket);
  if (closeBracket < 0 || text[closeBracket + 1] !== "(") return null;
  const dest = parseLinkDestination(text, closeBracket + 1);
  if (!dest) return null;
  const rawLabel = text.slice(openBracket + 1, closeBracket);
  if (isImage) {
    const alt = rawLabel.trim();
    return { html: escapeHtml(alt), plain: alt, next: dest.end };
  }
  const label = renderInline(rawLabel);
  const href = safeHref(dest.url);
  if (href) {
    const plain = label.plain.trim() === href || !label.plain.trim() ? href : `${label.plain} (${href})`;
    return { html: `<a href="${escapeHtml(href)}">${label.html}</a>`, plain, next: dest.end };
  }
  if (isFileHref(dest.url)) {
    const alreadyCode = label.html.startsWith("<code>") && label.html.endsWith("</code>");
    return {
      html: alreadyCode ? label.html : `<code>${label.html}</code>`,
      plain: label.plain,
      next: dest.end,
    };
  }
  return { html: label.html, plain: label.plain, next: dest.end };
}

function tryInlineEmphasis(text: string, i: number): { html: string; plain: string; next: number } | null {
  const specs: readonly { marker: string; tag: string; wordBound: boolean }[] = [
    { marker: "***", tag: "strong_em", wordBound: false },
    { marker: "**", tag: "strong", wordBound: false },
    { marker: "__", tag: "strong", wordBound: true },
    { marker: "~~", tag: "del", wordBound: false },
    { marker: "==", tag: "mark", wordBound: false },
    { marker: "*", tag: "em", wordBound: false },
    { marker: "_", tag: "em", wordBound: true },
  ];
  for (const spec of specs) {
    if (!text.startsWith(spec.marker, i)) continue;
    const prev = text[i - 1];
    const afterOpen = text[i + spec.marker.length];
    if (!afterOpen || afterOpen === " " || afterOpen === "\t" || afterOpen === "\n") continue;
    if (spec.wordBound && isWordChar(prev)) continue;
    if (spec.marker === "*" && text[i + 1] === "*") continue;
    if (spec.marker === "_" && text[i + 1] === "_") continue;
    const close = findDelimitedClose(text, i + spec.marker.length, spec.marker, spec.wordBound);
    if (close <= i + spec.marker.length) continue;
    const inner = renderInline(text.slice(i + spec.marker.length, close));
    const html = spec.tag === "strong_em"
      ? `<strong><em>${inner.html}</em></strong>`
      : `<${spec.tag}>${inner.html}</${spec.tag}>`;
    return { html, plain: inner.plain, next: close + spec.marker.length };
  }
  return null;
}

function tryInlineMath(text: string, i: number): { html: string; plain: string; next: number } | null {
  if (text[i] !== "$" || text[i + 1] === "$" || text[i + 1] === " " || text[i + 1] === "\t" || text[i + 1] === "\n") {
    return null;
  }
  for (let j = i + 1; j < text.length; j++) {
    if (text[j] === "\\" && j + 1 < text.length) { j++; continue; }
    if (text[j] === "\n") return null;
    if (text[j] === "$") {
      if (j === i + 1 || text[j - 1] === " " || text[j - 1] === "\t") return null;
      if (text[j + 1] !== undefined && /\d/.test(text[j + 1]!)) return null;
      const body = text.slice(i + 1, j);
      return { html: escapeHtml(body), plain: body, next: j + 1 };
    }
  }
  return null;
}

export function renderInline(text: string): FormattedCopyPayload {
  let html = "";
  let plain = "";
  let i = 0;
  while (i < text.length) {
    const ch = text[i]!;
    if (ch === "\\" && i + 1 < text.length && /[\\`*_{}[\]()#+\-.!|>~$=]/.test(text[i + 1]!)) {
      const lit = text[i + 1]!;
      html += escapeHtml(lit);
      plain += lit;
      i += 2;
      continue;
    }
    if (ch === "`") {
      let ticks = 0;
      while (i + ticks < text.length && text[i + ticks] === "`") ticks++;
      const close = findClosingCodeSpan(text, i + ticks, ticks);
      if (close >= 0) {
        let code = text.slice(i + ticks, close);
        if (code.length >= 2 && code.startsWith(" ") && code.endsWith(" ") && code.trim().length > 0) {
          code = code.slice(1, -1);
        }
        html += `<code>${escapeHtml(code)}</code>`;
        plain += code;
        i = close + ticks;
        continue;
      }
    }
    if (ch === "!" && text[i + 1] === "[") {
      const img = tryInlineLink(text, i, true);
      if (img) { html += img.html; plain += img.plain; i = img.next; continue; }
    }
    if (ch === "[") {
      const link = tryInlineLink(text, i, false);
      if (link) { html += link.html; plain += link.plain; i = link.next; continue; }
    }
    if (ch === "<") {
      const brMatch = /^<br\s*\/?>/i.exec(text.slice(i));
      if (brMatch) { html += "<br>"; plain += "\n"; i += brMatch[0].length; continue; }
      const auto = /^<(https?:\/\/[^\s<>]+|mailto:[^\s<>]+)>/i.exec(text.slice(i));
      if (auto) {
        const href = safeHref(auto[1]!);
        const display = auto[1]!.replace(/^mailto:/i, "");
        html += href ? `<a href="${escapeHtml(href)}">${escapeHtml(display)}</a>` : escapeHtml(display);
        plain += display;
        i += auto[0].length;
        continue;
      }
    }
    if (ch === "$") {
      const math = tryInlineMath(text, i);
      if (math) { html += math.html; plain += math.plain; i = math.next; continue; }
    }
    if (ch === "*" || ch === "_" || ch === "~" || ch === "=") {
      const em = tryInlineEmphasis(text, i);
      if (em) { html += em.html; plain += em.plain; i = em.next; continue; }
    }
    if (ch === "\n") {
      html += "<br>";
      plain += "\n";
      i++;
      continue;
    }
    html += escapeHtml(ch);
    plain += ch;
    i++;
  }
  return { html, plain };
}

function splitTableRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  const cells: string[] = [];
  let current = "";
  let inCodeTicks = 0;
  for (let i = 0; i < s.length; i++) {
    const ch = s[i]!;
    if (ch === "\\" && i + 1 < s.length && s[i + 1] === "|" && inCodeTicks === 0) {
      current += "|";
      i++;
      continue;
    }
    if (ch === "`") {
      let ticks = 0;
      while (i + ticks < s.length && s[i + ticks] === "`") ticks++;
      if (inCodeTicks === 0) inCodeTicks = ticks;
      else if (inCodeTicks === ticks) inCodeTicks = 0;
      current += s.slice(i, i + ticks);
      i += ticks - 1;
      continue;
    }
    if (ch === "|" && inCodeTicks === 0) {
      cells.push(current.trim());
      current = "";
      continue;
    }
    current += ch;
  }
  cells.push(current.trim());
  return cells;
}

function parseTableAlignments(line: string): ("left" | "center" | "right")[] | null {
  if (!line.includes("-")) return null;
  const cells = splitTableRow(line);
  if (cells.length === 0) return null;
  const aligns: ("left" | "center" | "right")[] = [];
  for (const cell of cells) {
    if (!/^:?-{3,}:?$/.test(cell)) return null;
    if (cell.startsWith(":") && cell.endsWith(":")) aligns.push("center");
    else if (cell.endsWith(":")) aligns.push("right");
    else aligns.push("left");
  }
  return aligns;
}

const TABLE_STYLE = ' style="border-collapse:collapse;margin:8px 0"';
function cellStyle(align: "left" | "center" | "right", header: boolean): string {
  const weight = header ? ";font-weight:600" : "";
  return ` style="border:1px solid #d0d7de;padding:6px 10px;text-align:${align}${weight}"`;
}

function parseTableBlock(lines: readonly string[], start: number): { html: string; plain: string; next: number } | null {
  const headerLine = lines[start] ?? "";
  const sepLine = lines[start + 1];
  if (!sepLine || !headerLine.includes("|")) return null;
  const aligns = parseTableAlignments(sepLine);
  if (!aligns) return null;
  const headers = splitTableRow(headerLine);
  if (headers.length !== aligns.length) return null;
  const rows: string[][] = [];
  let i = start + 2;
  while (i < lines.length) {
    const row = lines[i]!;
    if (!row.trim() || !row.includes("|")) break;
    if (/^\s*(?:```|~~~|#{1,6}\s|>)/.test(row)) break;
    rows.push(splitTableRow(row));
    i++;
  }
  const headRendered = headers.map((cell) => renderInline(cell));
  const headHtml = headRendered
    .map((c, idx) => `<th${cellStyle(aligns[idx] ?? "left", true)}>${c.html}</th>`)
    .join("");
  const plainLines = [headRendered.map((c) => c.plain).join("\t")];
  const bodyHtml = rows
    .map((cells) => {
      const rendered = aligns.map((_, idx) => renderInline(cells[idx] ?? ""));
      plainLines.push(rendered.map((c) => c.plain).join("\t"));
      return `<tr>${rendered.map((c, idx) => `<td${cellStyle(aligns[idx] ?? "left", false)}>${c.html}</td>`).join("")}</tr>`;
    })
    .join("");
  const html = `<table${TABLE_STYLE}><thead><tr>${headHtml}</tr></thead>${bodyHtml ? `<tbody>${bodyHtml}</tbody>` : ""}</table>`;
  return { html, plain: plainLines.join("\n"), next: i };
}

interface ListMarker {
  readonly indent: number;
  readonly ordered: boolean;
  readonly numberText: string;
  readonly task: "checked" | "unchecked" | null;
  readonly rest: string;
}

function matchListMarker(line: string): ListMarker | null {
  const match = /^(\s*)(?:([-*+])|(\d+[.)]))\s+(.*)$/.exec(line);
  if (!match) return null;
  const indent = match[1]!.replaceAll("\t", "  ").length;
  const ordered = match[3] !== undefined;
  let rest = match[4]!;
  let task: "checked" | "unchecked" | null = null;
  if (!ordered) {
    const taskMatch = /^\[([ xX])\]\s+(.*)$/.exec(rest);
    if (taskMatch) {
      task = taskMatch[1]!.toLowerCase() === "x" ? "checked" : "unchecked";
      rest = taskMatch[2]!;
    }
  }
  return { indent, ordered, numberText: match[3] ?? "", task, rest };
}

function parseListBlock(
  lines: readonly string[],
  start: number,
  depth = 0,
): { html: string; plain: string; next: number } {
  const first = matchListMarker(lines[start]!)!;
  const ordered = first.ordered;
  const baseIndent = first.indent;
  const htmlItems: string[] = [];
  const plainItems: string[] = [];
  const plainIndent = "  ".repeat(depth);
  let i = start;

  while (i < lines.length) {
    const marker = matchListMarker(lines[i]!);
    if (!marker || marker.indent < baseIndent) break;
    if (marker.indent === baseIndent && marker.ordered !== ordered) break;
    if (marker.indent > baseIndent) break;
    const itemLines = [marker.rest];
    i++;
    const nestedLines: string[] = [];
    while (i < lines.length) {
      const nextLine = lines[i]!;
      if (!nextLine.trim()) {
        const peek = lines[i + 1];
        const peekMarker = peek ? matchListMarker(peek) : null;
        if (peekMarker && peekMarker.indent >= baseIndent && (peekMarker.indent > baseIndent || peekMarker.ordered === ordered)) {
          i++;
          break;
        }
        break;
      }
      const nextMarker = matchListMarker(nextLine);
      if (nextMarker) {
        if (nextMarker.indent > baseIndent) {
          nestedLines.push(nextLine);
          i++;
          continue;
        }
        break;
      }
      const leadingSpaces = nextLine.match(/^\s*/)![0].replaceAll("\t", "  ").length;
      if (leadingSpaces > baseIndent) {
        if (nestedLines.length > 0) nestedLines.push(nextLine);
        else itemLines.push(nextLine.trim());
        i++;
        continue;
      }
      break;
    }
    const content = renderInline(itemLines.join("\n"));
    const taskPrefixHtml = marker.task === "checked" ? "☑ " : marker.task === "unchecked" ? "☐ " : "";
    const bulletPlain = ordered
      ? `${marker.numberText} `
      : marker.task === "checked"
      ? "☑ "
      : marker.task === "unchecked"
      ? "☐ "
      : "- ";
    let itemHtml = `${taskPrefixHtml}${content.html}`;
    let itemPlain = `${plainIndent}${bulletPlain}${content.plain}`;
    if (nestedLines.length > 0) {
      const nested = parseNestedListLines(nestedLines, depth + 1);
      itemHtml += nested.html;
      itemPlain += `\n${nested.plain}`;
    }
    htmlItems.push(`<li>${itemHtml}</li>`);
    plainItems.push(itemPlain);
  }
  const tag = ordered ? "ol" : "ul";
  return { html: `<${tag}>${htmlItems.join("")}</${tag}>`, plain: plainItems.join("\n"), next: i };
}

function parseNestedListLines(lines: readonly string[], depth: number): FormattedCopyPayload {
  const htmlParts: string[] = [];
  const plainParts: string[] = [];
  let i = 0;
  while (i < lines.length) {
    if (!lines[i]!.trim()) { i++; continue; }
    if (matchListMarker(lines[i]!)) {
      const block = parseListBlock(lines, i, depth);
      htmlParts.push(block.html);
      plainParts.push(block.plain);
      i = block.next;
    } else {
      const rendered = renderInline(lines[i]!.trim());
      htmlParts.push(`<p>${rendered.html}</p>`);
      plainParts.push(`${"  ".repeat(depth)}${rendered.plain}`);
      i++;
    }
  }
  return { html: htmlParts.join(""), plain: plainParts.join("\n") };
}

function isBlockStarter(lines: readonly string[], index: number): boolean {
  const line = lines[index]!;
  if (!line.trim()) return true;
  if (/^\s{0,3}(`{3,}|~{3,})/.test(line)) return true;
  if (/^\s{0,3}\$\$\s*$/.test(line)) return true;
  if (/^\s{0,3}#{1,6}\s+/.test(line)) return true;
  if (/^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line)) return true;
  if (/^\s{0,3}>/.test(line)) return true;
  if (matchListMarker(line)) return true;
  if (line.includes("|") && lines[index + 1] && parseTableAlignments(lines[index + 1]!)) return true;
  return false;
}

/** Convert GFM Markdown source into clean semantic HTML + readable plain text. */
export function formatMessageForCopy(markdown: string): FormattedCopyPayload {
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
  const htmlBlocks: string[] = [];
  const plainBlocks: string[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i]!;
    if (!line.trim()) { i++; continue; }

    // Fenced code block (``` or ~~~)
    const fence = /^\s{0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      const markerChar = fence[1]![0]!;
      const markerLen = fence[1]!.length;
      const codeLines: string[] = [];
      i++;
      while (i < lines.length) {
        const cur = lines[i]!;
        const closeRe = markerChar === "`"
          ? new RegExp(`^\\s{0,3}\`{${markerLen},}\\s*$`)
          : new RegExp(`^\\s{0,3}~{${markerLen},}\\s*$`);
        if (closeRe.test(cur)) { i++; break; }
        codeLines.push(cur);
        i++;
      }
      const code = codeLines.join("\n");
      htmlBlocks.push(`<pre><code>${escapeHtml(code)}</code></pre>`);
      plainBlocks.push(code);
      continue;
    }

    // Display math block ($$ ... $$)
    if (/^\s{0,3}\$\$\s*$/.test(line)) {
      const mathLines: string[] = [];
      i++;
      while (i < lines.length && !/^\s{0,3}\$\$\s*$/.test(lines[i]!)) {
        mathLines.push(lines[i]!);
        i++;
      }
      if (i < lines.length) i++;
      const math = mathLines.join("\n").trim();
      htmlBlocks.push(`<pre><code>${escapeHtml(math)}</code></pre>`);
      plainBlocks.push(math);
      continue;
    }

    // ATX Heading
    const heading = /^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+\s*)?$/.exec(line);
    if (heading) {
      const level = heading[1]!.length;
      const rendered = renderInline(heading[2]!);
      htmlBlocks.push(`<h${level}>${rendered.html}</h${level}>`);
      plainBlocks.push(rendered.plain);
      i++;
      continue;
    }

    // Horizontal rule
    if (/^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
      htmlBlocks.push("<hr>");
      plainBlocks.push("---");
      i++;
      continue;
    }

    // GFM Pipe Table
    const table = parseTableBlock(lines, i);
    if (table) {
      htmlBlocks.push(table.html);
      plainBlocks.push(table.plain);
      i = table.next;
      continue;
    }

    // Blockquote
    if (/^\s{0,3}>/.test(line)) {
      const quoted: string[] = [];
      while (i < lines.length && /^\s{0,3}>/.test(lines[i]!)) {
        quoted.push(lines[i]!.replace(/^\s{0,3}>\s?/, ""));
        i++;
      }
      const inner = formatMessageForCopy(quoted.join("\n"));
      htmlBlocks.push(`<blockquote>${inner.html}</blockquote>`);
      plainBlocks.push(inner.plain.split("\n").map((l) => (l ? `> ${l}` : ">")).join("\n"));
      continue;
    }

    // List
    if (matchListMarker(line)) {
      const list = parseListBlock(lines, i, 0);
      htmlBlocks.push(list.html);
      plainBlocks.push(list.plain);
      i = list.next;
      continue;
    }

    // Paragraph (including trailing two-space / backslash hard line breaks)
    const paraLines: string[] = [];
    while (i < lines.length && !isBlockStarter(lines, i)) {
      paraLines.push(lines[i]!.replace(/(?: {2}|\\)$/, ""));
      i++;
    }
    const rendered = renderInline(paraLines.join("\n"));
    htmlBlocks.push(`<p>${rendered.html}</p>`);
    plainBlocks.push(rendered.plain);
  }

  return {
    html: htmlBlocks.join("\n"),
    plain: plainBlocks.join("\n\n"),
  };
}
