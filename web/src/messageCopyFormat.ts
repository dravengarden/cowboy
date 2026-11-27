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
import {
  chargeBudget,
  createBudget,
  escapeHtml,
  type FormattedCopyPayload,
  MAX_NESTING_DEPTH,
  type ParseBudget,
  renderInline,
  renderInlineInternal,
} from "./messageCopyInline";

export { type FormattedCopyPayload, renderInline };

export interface CopyableMessage {
  readonly markdown: string;
  readonly role: "assistant" | "user";
}

const MAX_TABLE_CELLS = 4000;
const MAX_TABLE_OUTPUT_CHARS = 512_000;

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

type TableAlign = "left" | "center" | "right";

function isEscapedAt(text: string, index: number): boolean {
  let backslashes = 0;
  for (let j = index - 1; j >= 0 && text[j] === "\\"; j--) backslashes++;
  return backslashes % 2 === 1;
}

/** Per GFM §4.10, `\|` escapes a literal pipe everywhere in a table row,
 *  including inside inline code spans. */
function splitTableRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !isEscapedAt(s, s.length - 1)) s = s.slice(0, -1);
  const cells: string[] = [];
  let current = "";
  for (let i = 0; i < s.length; i++) {
    const ch = s[i]!;
    if (ch === "\\" && i + 1 < s.length && s[i + 1] === "|") {
      current += "|";
      i++;
      continue;
    }
    if (ch === "|") {
      cells.push(current.trim());
      current = "";
      continue;
    }
    current += ch;
  }
  cells.push(current.trim());
  return cells;
}

/** Accept valid GFM delimiter cells of any dash length (`| - | - |`, `:-:`, etc.). */
function parseTableAlignments(line: string): TableAlign[] | null {
  if (!line.includes("-") || !line.includes("|")) return null;
  const cells = splitTableRow(line);
  if (cells.length === 0) return null;
  const aligns: TableAlign[] = [];
  for (const cell of cells) {
    if (!/^:?-{1,}:?$/.test(cell)) return null;
    if (cell.startsWith(":") && cell.endsWith(":")) aligns.push("center");
    else if (cell.endsWith(":")) aligns.push("right");
    else aligns.push("left");
  }
  return aligns;
}

function matchTableHeader(
  lines: readonly string[],
  start: number,
): { headers: string[]; aligns: TableAlign[] } | null {
  const headerLine = lines[start] ?? "";
  const sepLine = lines[start + 1];
  if (!sepLine || !headerLine.includes("|")) return null;
  const aligns = parseTableAlignments(sepLine);
  if (!aligns) return null;
  const headers = splitTableRow(headerLine);
  if (headers.length !== aligns.length) return null;
  return { headers, aligns };
}

function parseFenceOpener(line: string): { markerChar: "`" | "~"; markerLen: number } | null {
  const match = /^\s{0,3}(`{3,}|~{3,})(.*)$/.exec(line);
  if (!match) return null;
  const markerChar = match[1]![0] as "`" | "~";
  const info = match[2] ?? "";
  if (markerChar === "`" && info.includes("`")) return null;
  return { markerChar, markerLen: match[1]!.length };
}

function isFenceClose(line: string, opener: { markerChar: "`" | "~"; markerLen: number }): boolean {
  const trimmed = line.replace(/^ {0,3}/, "");
  let count = 0;
  while (count < trimmed.length && trimmed[count] === opener.markerChar) count++;
  return count >= opener.markerLen && trimmed.slice(count).trim().length === 0;
}

function findBlockMathClose(
  lines: readonly string[],
  start: number,
  budget: ParseBudget,
): { closeIndex: number; kind: "$$" | "\\[" } | null {
  const opener = /^\s{0,3}(\$\$|\\\[)\s*$/.exec(lines[start] ?? "");
  if (!opener) return null;
  const kind = opener[1] as "$$" | "\\[";
  const closeRe = kind === "$$" ? /^\s{0,3}\$\$\s*$/ : /^\s{0,3}\\\]\s*$/;
  for (let j = start + 1; j < lines.length; j++) {
    if (!chargeBudget(budget)) return null;
    if (closeRe.test(lines[j]!)) return { closeIndex: j, kind };
  }
  return null;
}

const TABLE_STYLE = ' style="border-collapse:collapse;margin:8px 0"';
function cellStyle(align: TableAlign, header: boolean): string {
  const weight = header ? ";font-weight:600" : "";
  return ` style="border:1px solid #d0d7de;padding:6px 10px;text-align:${align}${weight}"`;
}

/** Per GFM §4.10, a table body continues until a blank line or another
 *  block-level starter (sparse 1-cell rows without `|` remain part of the table). */
function isTableRowTerminator(lines: readonly string[], index: number, budget: ParseBudget): boolean {
  const line = lines[index]!;
  if (!line.trim()) return true;
  if (parseFenceOpener(line)) return true;
  if (/^\s{0,3}(?:\$\$.+\$\$|\\\[.+\\\])\s*$/.test(line)) return true;
  if (findBlockMathClose(lines, index, budget) !== null) return true;
  if (/^\s{0,3}#{1,6}\s+/.test(line)) return true;
  if (/^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line)) return true;
  if (/^\s{0,3}>/.test(line)) return true;
  if (matchListMarker(line)) return true;
  if (matchTableHeader(lines, index)) return true;
  return false;
}

function fallbackEscapedLines(lines: readonly string[], start: number, end: number): { html: string; plain: string; next: number } {
  const raw = lines.slice(start, end).join("\n");
  return { html: `<p>${escapeHtml(raw).replaceAll("\n", "<br>")}</p>`, plain: raw, next: end };
}

function parseTableBlock(
  lines: readonly string[],
  start: number,
  depth: number,
  budget: ParseBudget,
): { html: string; plain: string; next: number } | null {
  const matched = matchTableHeader(lines, start);
  if (!matched) return null;
  const { headers, aligns } = matched;
  const rows: string[][] = [];
  let i = start + 2;
  let cellCount = aligns.length;
  let oversized = cellCount > MAX_TABLE_CELLS;

  while (i < lines.length) {
    if (!chargeBudget(budget)) { oversized = true; break; }
    if (isTableRowTerminator(lines, i, budget)) break;
    cellCount += aligns.length;
    if (cellCount > MAX_TABLE_CELLS || !chargeBudget(budget, aligns.length)) {
      oversized = true;
      i++;
      while (i < lines.length && lines[i]!.trim() && !isTableRowTerminator(lines, i, budget)) i++;
      break;
    }
    rows.push(splitTableRow(lines[i]!));
    i++;
  }
  if (oversized || budget.exceeded) return fallbackEscapedLines(lines, start, i);

  const headRendered = headers.map((cell) => renderInlineInternal(cell, depth + 1, budget));
  if (budget.exceeded) return fallbackEscapedLines(lines, start, i);
  const headHtml = headRendered
    .map((c, idx) => `<th${cellStyle(aligns[idx] ?? "left", true)}>${c.html}</th>`)
    .join("");
  const plainLines = [headRendered.map((c) => c.plain).join("\t")];
  const bodyRows: string[] = [];
  let htmlChars = headHtml.length + 64;

  for (const cells of rows) {
    const rendered = aligns.map((_, idx) => renderInlineInternal(cells[idx] ?? "", depth + 1, budget));
    const rowHtml = `<tr>${rendered.map((c, idx) => `<td${cellStyle(aligns[idx] ?? "left", false)}>${c.html}</td>`).join("")}</tr>`;
    htmlChars += rowHtml.length;
    if (budget.exceeded || htmlChars > MAX_TABLE_OUTPUT_CHARS) {
      return fallbackEscapedLines(lines, start, i);
    }
    plainLines.push(rendered.map((c) => c.plain).join("\t"));
    bodyRows.push(rowHtml);
  }
  const bodyHtml = bodyRows.join("");
  const html = `<table${TABLE_STYLE}><thead><tr>${headHtml}</tr></thead>${bodyHtml ? `<tbody>${bodyHtml}</tbody>` : ""}</table>`;
  return { html, plain: plainLines.join("\n"), next: i };
}

function measureLeadingCols(line: string): number {
  let cols = 0;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i]!;
    if (ch === " ") cols += 1;
    else if (ch === "\t") cols += 4 - (cols % 4);
    else break;
  }
  return cols;
}

function stripIndentCols(line: string, maxCols: number): string {
  let cols = 0;
  let i = 0;
  while (i < line.length && cols < maxCols) {
    const ch = line[i]!;
    if (ch === " ") {
      cols += 1;
      i++;
    } else if (ch === "\t") {
      const step = 4 - (cols % 4);
      if (cols + step > maxCols) return " ".repeat(cols + step - maxCols) + line.slice(i + 1);
      cols += step;
      i++;
    } else {
      break;
    }
  }
  return line.slice(i);
}

interface ListMarker {
  readonly indent: number;
  readonly contentIndent: number;
  readonly ordered: boolean;
  readonly numberText: string;
  readonly task: "checked" | "unchecked" | null;
  readonly rest: string;
}

function matchListMarker(line: string): ListMarker | null {
  const match = /^([ \t]*)(?:([-*+])|(\d+[.)]))(?:([ \t]+)(.*)|[ \t]*)$/.exec(line);
  if (!match) return null;
  const indent = measureLeadingCols(match[1]!);
  const markerToken = match[2] ?? match[3]!;
  const ordered = match[3] !== undefined;
  const afterWs = match[4] ?? "";
  let rest = match[5] ?? "";
  let gapCols = afterWs ? measureLeadingCols(afterWs) : 1;
  if (gapCols > 4) {
    rest = " ".repeat(gapCols - 1) + rest;
    gapCols = 1;
  }
  const contentIndent = indent + markerToken.length + gapCols;
  let task: "checked" | "unchecked" | null = null;
  if (!ordered) {
    const taskMatch = /^\[([ xX])\](?:\s+(.*)|)$/.exec(rest);
    if (taskMatch) {
      task = taskMatch[1]!.toLowerCase() === "x" ? "checked" : "unchecked";
      rest = taskMatch[2] ?? "";
    }
  }
  return { indent, contentIndent, ordered, numberText: match[3] ?? "", task, rest };
}

function collectListItemLines(
  lines: readonly string[],
  start: number,
  marker: ListMarker,
  ordered: boolean,
): { bodyLines: string[]; next: number; hasBlank: boolean } {
  const rawContinuations: string[] = [];
  let minContIndent = marker.contentIndent;
  let hasBlank = false;
  let i = start + 1;
  let fence = parseFenceOpener(marker.rest);

  while (i < lines.length) {
    const nextLine = lines[i]!;
    if (!nextLine.trim()) {
      let j = i;
      while (j < lines.length && !lines[j]!.trim()) j++;
      if (j >= lines.length) break;
      const peek = lines[j]!;
      if (measureLeadingCols(peek) > marker.indent) {
        if (!fence) hasBlank = true;
        for (let k = i; k < j; k++) rawContinuations.push("");
        i = j;
        continue;
      }
      const peekMarker = matchListMarker(peek);
      if (peekMarker && peekMarker.indent === marker.indent && peekMarker.ordered === ordered) {
        i = j;
      }
      break;
    }

    // List item boundary always terminates even an unclosed inner fence.
    const leading = measureLeadingCols(nextLine);
    if (leading <= marker.indent) break;

    if (fence) {
      const stripped = stripIndentCols(nextLine, Math.min(leading, minContIndent));
      if (isFenceClose(stripped, fence)) fence = null;
      rawContinuations.push(nextLine);
      i++;
      continue;
    }

    minContIndent = Math.min(minContIndent, leading);
    const tentative = stripIndentCols(nextLine, Math.min(leading, minContIndent));
    const opener = parseFenceOpener(tentative);
    if (opener) fence = opener;
    rawContinuations.push(nextLine);
    i++;
  }

  const stripCols = Math.max(marker.indent + 1, minContIndent);
  const bodyLines = [
    marker.rest,
    ...rawContinuations.map((raw) => (raw.trim() ? stripIndentCols(raw, stripCols) : "")),
  ];
  return { bodyLines, next: i, hasBlank };
}

function formatListItemInnerHtml(innerHtml: string, hasBlank: boolean): string {
  const singleP = /^<p>([\s\S]*)<\/p>$/.exec(innerHtml);
  if (singleP && !singleP[1]!.includes("</p>")) return singleP[1]!;
  if (!hasBlank) {
    const leadingP = /^<p>([\s\S]*?)<\/p>\n(<(?:ul|ol)>[\s\S]*)$/.exec(innerHtml);
    if (leadingP) return `${leadingP[1]!}${leadingP[2]!}`;
  }
  return innerHtml;
}

function parseListBlock(
  lines: readonly string[],
  start: number,
  depth: number,
  budget: ParseBudget,
): { html: string; plain: string; next: number } {
  const first = matchListMarker(lines[start]!)!;
  const ordered = first.ordered;
  const baseIndent = first.indent;
  const htmlItems: string[] = [];
  const plainItems: string[] = [];
  let i = start;

  while (i < lines.length) {
    if (!chargeBudget(budget)) break;
    const marker = matchListMarker(lines[i]!);
    if (!marker || marker.indent !== baseIndent || marker.ordered !== ordered) break;
    const { bodyLines, next, hasBlank } = collectListItemLines(lines, i, marker, ordered);
    i = next;
    const inner = formatBlocks(bodyLines, depth + 1, budget);
    const taskPrefixHtml = marker.task === "checked" ? "☑ " : marker.task === "unchecked" ? "☐ " : "";
    const bulletPlain = ordered
      ? `${marker.numberText} `
      : marker.task === "checked"
      ? "☑ "
      : marker.task === "unchecked"
      ? "☐ "
      : "- ";
    const itemHtml = `${taskPrefixHtml}${formatListItemInnerHtml(inner.html, hasBlank)}`;
    const innerPlainLines = inner.plain.split("\n");
    const firstPlain = `${bulletPlain}${innerPlainLines[0] ?? ""}`;
    const restPlain = innerPlainLines.slice(1).map((l) => (l.length > 0 ? `  ${l}` : ""));
    htmlItems.push(`<li>${itemHtml}</li>`);
    plainItems.push([firstPlain, ...restPlain].join("\n"));
  }
  const tag = ordered ? "ol" : "ul";
  return { html: `<${tag}>${htmlItems.join("")}</${tag}>`, plain: plainItems.join("\n"), next: i };
}

function isBlockStarter(lines: readonly string[], index: number, budget: ParseBudget): boolean {
  const line = lines[index]!;
  if (!line.trim()) return true;
  if (parseFenceOpener(line)) return true;
  if (/^\s{0,3}(?:\$\$.+\$\$|\\\[.+\\\])\s*$/.test(line)) return true;
  if (findBlockMathClose(lines, index, budget) !== null) return true;
  if (/^\s{0,3}#{1,6}\s+/.test(line)) return true;
  if (/^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line)) return true;
  if (/^\s{0,3}>/.test(line)) return true;
  if (matchListMarker(line)) return true;
  if (matchTableHeader(lines, index)) return true;
  return false;
}

function formatBlocks(
  lines: readonly string[],
  depth: number,
  budget: ParseBudget,
): FormattedCopyPayload {
  if (depth >= MAX_NESTING_DEPTH || budget.exceeded) {
    const raw = lines.join("\n").trim();
    if (!raw) return { html: "", plain: "" };
    return { html: `<p>${escapeHtml(raw).replaceAll("\n", "<br>")}</p>`, plain: raw };
  }
  const htmlBlocks: string[] = [];
  const plainBlocks: string[] = [];
  let i = 0;

  while (i < lines.length) {
    if (!chargeBudget(budget)) {
      const rest = lines.slice(i).join("\n").trim();
      if (rest) {
        htmlBlocks.push(`<p>${escapeHtml(rest).replaceAll("\n", "<br>")}</p>`);
        plainBlocks.push(rest);
      }
      break;
    }
    const line = lines[i]!;
    if (!line.trim()) { i++; continue; }

    const fence = parseFenceOpener(line);
    if (fence) {
      const codeLines: string[] = [];
      i++;
      while (i < lines.length) {
        if (!chargeBudget(budget)) break;
        const cur = lines[i]!;
        if (isFenceClose(cur, fence)) { i++; break; }
        codeLines.push(cur);
        i++;
      }
      const code = codeLines.join("\n");
      htmlBlocks.push(`<pre><code>${escapeHtml(code)}</code></pre>`);
      plainBlocks.push(code);
      continue;
    }

    const singleMath = /^\s{0,3}(?:\$\$(.+?)\$\$|\\\[(.+?)\\\])\s*$/.exec(line);
    if (singleMath) {
      const math = (singleMath[1] ?? singleMath[2] ?? "").trim();
      htmlBlocks.push(`<pre><code>${escapeHtml(math)}</code></pre>`);
      plainBlocks.push(math);
      i++;
      continue;
    }

    const blockMath = findBlockMathClose(lines, i, budget);
    if (blockMath) {
      const math = lines.slice(i + 1, blockMath.closeIndex).join("\n").trim();
      htmlBlocks.push(`<pre><code>${escapeHtml(math)}</code></pre>`);
      plainBlocks.push(math);
      i = blockMath.closeIndex + 1;
      continue;
    }

    const heading = /^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+\s*)?$/.exec(line);
    if (heading) {
      const level = heading[1]!.length;
      const rendered = renderInlineInternal(heading[2]!, depth + 1, budget);
      htmlBlocks.push(`<h${level}>${rendered.html}</h${level}>`);
      plainBlocks.push(rendered.plain);
      i++;
      continue;
    }

    if (/^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
      htmlBlocks.push("<hr>");
      plainBlocks.push("---");
      i++;
      continue;
    }

    const table = parseTableBlock(lines, i, depth, budget);
    if (table) {
      htmlBlocks.push(table.html);
      plainBlocks.push(table.plain);
      i = table.next;
      continue;
    }

    if (/^\s{0,3}>/.test(line)) {
      const quoted: string[] = [];
      while (i < lines.length && /^\s{0,3}>/.test(lines[i]!)) {
        if (!chargeBudget(budget)) break;
        quoted.push(lines[i]!.replace(/^\s{0,3}>\s?/, ""));
        i++;
      }
      const inner = formatBlocks(quoted, depth + 1, budget);
      htmlBlocks.push(`<blockquote>${inner.html}</blockquote>`);
      plainBlocks.push(inner.plain.split("\n").map((l) => (l ? `> ${l}` : ">")).join("\n"));
      continue;
    }

    if (matchListMarker(line)) {
      const list = parseListBlock(lines, i, depth, budget);
      htmlBlocks.push(list.html);
      plainBlocks.push(list.plain);
      i = list.next;
      continue;
    }

    const paraLines: string[] = [];
    while (i < lines.length && (paraLines.length === 0 || !isBlockStarter(lines, i, budget))) {
      if (!chargeBudget(budget)) break;
      paraLines.push(lines[i]!);
      i++;
    }
    const rendered = renderInlineInternal(paraLines.join("\n"), depth + 1, budget);
    htmlBlocks.push(`<p>${rendered.html}</p>`);
    plainBlocks.push(rendered.plain);
  }

  return {
    html: htmlBlocks.join("\n"),
    plain: plainBlocks.join("\n\n"),
  };
}

/** Convert GFM Markdown source into clean semantic HTML + readable plain text. */
export function formatMessageForCopy(markdown: string): FormattedCopyPayload {
  const normalized = markdown.replace(/\r\n?/g, "\n");
  const result = formatBlocks(normalized.split("\n"), 0, createBudget());
  if (!result.plain.trim() && markdown.trim()) {
    const fallback = markdown.trim();
    return { html: `<p>${escapeHtml(fallback).replaceAll("\n", "<br>")}</p>`, plain: fallback };
  }
  return result;
}
