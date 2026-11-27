// Bounded, linear-time inline GFM Markdown serializer for message Copy.
// Separated from block formatting so both modules stay concise and focused.

export interface FormattedCopyPayload {
  readonly html: string;
  readonly plain: string;
}

export interface ParseBudget {
  steps: number;
  exceeded: boolean;
}

export const MAX_NESTING_DEPTH = 12;
const MAX_PARSE_STEPS = 200_000;
const ESCAPABLE_PUNCT = /[\\`*_{}[\]()#+\-.!|>~$=]/;

export function createBudget(): ParseBudget {
  return { steps: 0, exceeded: false };
}

export function chargeBudget(budget: ParseBudget, cost = 1): boolean {
  budget.steps += cost;
  if (budget.steps > MAX_PARSE_STEPS) {
    budget.exceeded = true;
    return false;
  }
  return true;
}

export function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function decodeHtmlEntities(text: string): string {
  return text.replace(/&(?:#(\d+)|#x([0-9a-fA-F]+)|(amp|lt|gt|quot|apos));/g, (match, dec, hex, named) => {
    if (named) {
      if (named === "amp") return "&";
      if (named === "lt") return "<";
      if (named === "gt") return ">";
      if (named === "quot") return '"';
      if (named === "apos") return "'";
    }
    const code = dec ? Number.parseInt(dec, 10) : hex ? Number.parseInt(hex, 16) : -1;
    if (code <= 0 || code > 0x10ffff) return match;
    try {
      return String.fromCodePoint(code);
    } catch {
      return match;
    }
  });
}

function unescapeDestination(raw: string): string {
  const unescaped = raw.replace(/\\([\\!"#$%&'()*+,\-./:;<=>?@[\]^_`{|}~])/g, "$1");
  return decodeHtmlEntities(unescaped);
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

function findClosingCodeSpan(
  text: string,
  from: number,
  ticks: number,
  deadTicks: Map<number, number>,
  budget: ParseBudget,
): number {
  const deadAt = deadTicks.get(ticks);
  if (deadAt !== undefined && from >= deadAt) return -1;
  let i = from;
  while (i < text.length) {
    if (!chargeBudget(budget)) return -1;
    if (text[i] === "`") {
      let count = 0;
      while (i + count < text.length && text[i + count] === "`") count++;
      if (count === ticks) return i;
      i += count;
    } else {
      i++;
    }
  }
  deadTicks.set(ticks, from);
  return -1;
}

function skipInlineWs(text: string, from: number): number {
  let i = from;
  let sawNewline = false;
  while (i < text.length) {
    const ch = text[i]!;
    if (ch === " " || ch === "\t") { i++; continue; }
    if (ch === "\n" && !sawNewline) { sawNewline = true; i++; continue; }
    break;
  }
  return i;
}

function skipLinkTitle(text: string, from: number): number {
  const open = text[from];
  if (open !== '"' && open !== "'" && open !== "(") return -1;
  const close = open === "(" ? ")" : open;
  let i = from + 1;
  while (i < text.length) {
    const ch = text[i]!;
    if (ch === "\\" && i + 1 < text.length) { i += 2; continue; }
    if (ch === close) return i + 1;
    if (open === "(" && ch === "(") return -1;
    if (ch === "\n" && text[i + 1] === "\n") return -1;
    i++;
  }
  return -1;
}

function parseLinkDestination(text: string, openParen: number): { url: string; end: number } | null {
  let i = skipInlineWs(text, openParen + 1);
  if (i >= text.length) return null;
  let rawUrl = "";
  if (text[i] === "<") {
    i++;
    const start = i;
    while (i < text.length) {
      const ch = text[i]!;
      if (ch === "\\" && i + 1 < text.length) { i += 2; continue; }
      if (ch === "<" || ch === "\n") return null;
      if (ch === ">") break;
      i++;
    }
    if (i >= text.length || text[i] !== ">") return null;
    rawUrl = text.slice(start, i);
    i++;
  } else {
    const start = i;
    let depth = 0;
    while (i < text.length) {
      const ch = text[i]!;
      if (ch === "\\" && i + 1 < text.length) { i += 2; continue; }
      if (ch === " " || ch === "\t" || ch === "\n" || ch.charCodeAt(0) < 0x20) break;
      if (ch === "(") {
        depth++;
      } else if (ch === ")") {
        if (depth === 0) break;
        depth--;
      }
      i++;
    }
    if (depth !== 0 || i === start) return null;
    rawUrl = text.slice(start, i);
  }
  const afterUrl = i;
  i = skipInlineWs(text, i);
  if (text[i] === ")") return { url: unescapeDestination(rawUrl), end: i + 1 };
  if (i === afterUrl) return null;
  const afterTitle = skipLinkTitle(text, i);
  if (afterTitle < 0) return null;
  i = skipInlineWs(text, afterTitle);
  if (text[i] !== ")") return null;
  return { url: unescapeDestination(rawUrl), end: i + 1 };
}

function computeBracketMatches(
  text: string,
  deadTicks: Map<number, number>,
  budget: ParseBudget,
): Map<number, number> {
  const matches = new Map<number, number>();
  const stack: number[] = [];
  for (let i = 0; i < text.length; i++) {
    if (!chargeBudget(budget)) break;
    const ch = text[i]!;
    if (ch === "\\" && i + 1 < text.length) { i++; continue; }
    if (ch === "`") {
      let ticks = 0;
      while (i + ticks < text.length && text[i + ticks] === "`") ticks++;
      const close = findClosingCodeSpan(text, i + ticks, ticks, deadTicks, budget);
      i = close >= 0 ? close + ticks - 1 : i + ticks - 1;
      continue;
    }
    if (ch === "[") stack.push(i);
    else if (ch === "]" && stack.length > 0) matches.set(stack.pop()!, i);
  }
  return matches;
}

function isWordChar(ch: string | undefined): boolean {
  return ch !== undefined && /[\p{L}\p{N}_]/u.test(ch);
}

interface InlineState {
  readonly brackets: Map<number, number>;
  readonly deadTicks: Map<number, number>;
  readonly deadMarkers: Map<string, number>;
  readonly budget: ParseBudget;
  readonly depth: number;
}

function findDelimitedClose(
  text: string,
  from: number,
  marker: string,
  requireWordBoundary: boolean,
  state: InlineState,
): number {
  const key = `${marker}:${requireWordBoundary ? "1" : "0"}`;
  const deadAt = state.deadMarkers.get(key);
  if (deadAt !== undefined && from >= deadAt) return -1;
  let i = from;
  while (i < text.length) {
    if (!chargeBudget(state.budget)) return -1;
    if (text[i] === "\\" && i + 1 < text.length) { i += 2; continue; }
    if (text[i] === "`") {
      let ticks = 0;
      while (i + ticks < text.length && text[i + ticks] === "`") ticks++;
      const close = findClosingCodeSpan(text, i + ticks, ticks, state.deadTicks, state.budget);
      i = close >= 0 ? close + ticks : i + ticks;
      continue;
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
  state.deadMarkers.set(key, from);
  return -1;
}

function tryInlineLink(
  text: string,
  start: number,
  isImage: boolean,
  state: InlineState,
): { html: string; plain: string; next: number } | null {
  const openBracket = isImage ? start + 1 : start;
  const closeBracket = state.brackets.get(openBracket);
  if (closeBracket === undefined || text[closeBracket + 1] !== "(") return null;
  const dest = parseLinkDestination(text, closeBracket + 1);
  if (!dest) return null;
  const rawLabel = text.slice(openBracket + 1, closeBracket);
  if (isImage) {
    const alt = rawLabel.trim() || "[Image]";
    return { html: escapeHtml(alt), plain: alt, next: dest.end };
  }
  const label = renderInlineInternal(rawLabel, state.depth + 1, state.budget);
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

const EMPHASIS_SPECS: readonly { marker: string; tag: string; wordBound: boolean }[] = [
  { marker: "***", tag: "strong_em", wordBound: false },
  { marker: "**", tag: "strong", wordBound: false },
  { marker: "__", tag: "strong", wordBound: true },
  { marker: "~~", tag: "del", wordBound: false },
  { marker: "==", tag: "mark", wordBound: false },
  { marker: "*", tag: "em", wordBound: false },
  { marker: "_", tag: "em", wordBound: true },
];

function tryInlineEmphasis(
  text: string,
  i: number,
  state: InlineState,
): { html: string; plain: string; next: number } | null {
  for (const spec of EMPHASIS_SPECS) {
    if (!text.startsWith(spec.marker, i)) continue;
    const prev = text[i - 1];
    const afterOpen = text[i + spec.marker.length];
    if (!afterOpen || afterOpen === " " || afterOpen === "\t" || afterOpen === "\n") continue;
    if (spec.wordBound && isWordChar(prev)) continue;
    if (spec.marker === "*" && text[i + 1] === "*") continue;
    if (spec.marker === "_" && text[i + 1] === "_") continue;
    const close = findDelimitedClose(text, i + spec.marker.length, spec.marker, spec.wordBound, state);
    if (close <= i + spec.marker.length) continue;
    const inner = renderInlineInternal(text.slice(i + spec.marker.length, close), state.depth + 1, state.budget);
    const html = spec.tag === "strong_em"
      ? `<strong><em>${inner.html}</em></strong>`
      : `<${spec.tag}>${inner.html}</${spec.tag}>`;
    return { html, plain: inner.plain, next: close + spec.marker.length };
  }
  return null;
}

function tryLatexInlineMath(
  text: string,
  i: number,
  budget: ParseBudget,
): { html: string; plain: string; next: number } | null {
  const pair = text.slice(i, i + 2);
  if (pair !== "\\(" && pair !== "\\[") return null;
  const close = pair === "\\(" ? "\\)" : "\\]";
  for (let j = i + 2; j < text.length - 1; j++) {
    if (!chargeBudget(budget)) return null;
    if (text[j] === "\\" && text[j + 1] === "\\") { j++; continue; }
    if (text[j] === close[0] && text[j + 1] === close[1]) {
      const body = text.slice(i + 2, j).trim();
      if (!body) return null;
      return { html: escapeHtml(body), plain: body, next: j + 2 };
    }
  }
  return null;
}

function tryInlineMath(
  text: string,
  i: number,
  budget: ParseBudget,
): { html: string; plain: string; next: number } | null {
  if (text[i] === "\\") return tryLatexInlineMath(text, i, budget);
  if (text[i] !== "$") return null;
  if (text[i + 1] === "$") {
    for (let j = i + 2; j < text.length - 1; j++) {
      if (!chargeBudget(budget)) return null;
      if (text[j] === "\\" && j + 1 < text.length) { j++; continue; }
      if (text[j] === "$" && text[j + 1] === "$") {
        const body = text.slice(i + 2, j).trim();
        if (!body) return null;
        return { html: escapeHtml(body), plain: body, next: j + 2 };
      }
    }
    return null;
  }
  if (text[i + 1] === "\n") return null;
  const spaced = text[i + 1] === " " || text[i + 1] === "\t";
  for (let j = i + 1; j < text.length; j++) {
    if (!chargeBudget(budget)) return null;
    if (text[j] === "\\" && j + 1 < text.length) { j++; continue; }
    if (text[j] === "\n") return null;
    if (text[j] === "$") {
      if (j === i + 1) return null;
      if (text[j + 1] !== undefined && /\d/.test(text[j + 1]!)) return null;
      const body = text.slice(i + 1, j).trim();
      if (!body) return null;
      const trailingSpace = text[j - 1] === " " || text[j - 1] === "\t";
      if ((spaced || trailingSpace) && !(/[\\=^_+\-*/]/.test(body))) return null;
      return { html: escapeHtml(body), plain: body, next: j + 1 };
    }
  }
  return null;
}

export function renderInlineInternal(text: string, depth: number, budget: ParseBudget): FormattedCopyPayload {
  if (depth >= MAX_NESTING_DEPTH || budget.exceeded) {
    return { html: escapeHtml(text).replaceAll("\n", "<br>"), plain: text };
  }
  const deadTicks = new Map<number, number>();
  const state: InlineState = {
    brackets: text.includes("[") && text.includes("]")
      ? computeBracketMatches(text, deadTicks, budget)
      : new Map(),
    deadTicks,
    deadMarkers: new Map(),
    budget,
    depth,
  };
  let html = "";
  let plain = "";
  let i = 0;
  while (i < text.length) {
    if (!chargeBudget(budget)) {
      const rest = text.slice(i);
      return { html: html + escapeHtml(rest).replaceAll("\n", "<br>"), plain: plain + rest };
    }
    const ch = text[i]!;
    if (ch === "$" || (ch === "\\" && (text[i + 1] === "(" || text[i + 1] === "["))) {
      const math = tryInlineMath(text, i, budget);
      if (math) { html += math.html; plain += math.plain; i = math.next; continue; }
    }
    if (ch === "\\" && i + 1 < text.length && ESCAPABLE_PUNCT.test(text[i + 1]!)) {
      const lit = text[i + 1]!;
      html += escapeHtml(lit);
      plain += lit;
      i += 2;
      continue;
    }
    if (ch === "`") {
      let ticks = 0;
      while (i + ticks < text.length && text[i + ticks] === "`") ticks++;
      const close = findClosingCodeSpan(text, i + ticks, ticks, deadTicks, budget);
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
      const literalTicks = text.slice(i, i + ticks);
      html += literalTicks;
      plain += literalTicks;
      i += ticks;
      continue;
    }
    if (ch === "!" && text[i + 1] === "[") {
      const img = tryInlineLink(text, i, true, state);
      if (img) { html += img.html; plain += img.plain; i = img.next; continue; }
    }
    if (ch === "[") {
      const link = tryInlineLink(text, i, false, state);
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
    if (ch === "*" || ch === "_" || ch === "~" || ch === "=") {
      const em = tryInlineEmphasis(text, i, state);
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

export function renderInline(text: string): FormattedCopyPayload {
  return renderInlineInternal(text, 0, createBudget());
}
