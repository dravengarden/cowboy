// Structural size ratchet (docs/structural-refactor-plan-2026-11.md, slice 0.1).
//
// Measures production source files, functions and React hook calls and
// compares the ones over their limit with tools/structure-baseline.json:
//
//   - an entity over its limit must be recorded in the baseline and may not
//     grow past its recorded size;
//   - an entity may shrink freely without touching the baseline;
//   - `--update` lowers recorded sizes, drops entities that fell under their
//     limit, and admits a new over-limit entity (for example a component moved
//     unchanged into its own file) only while the recorded total of that kind
//     in the same area does not grow. Allowances therefore follow pure moves,
//     and the aggregate can only decrease.
//
// The lexer is comment- and string-aware but deliberately small: it is a
// deterministic measure for a ratchet, not a parser. Run from the repository
// root through the pinned shell:
//
//   bun tools/check-structure.ts            check against the baseline
//   bun tools/check-structure.ts --update   lower and re-admit the baseline
//   bun tools/check-structure.ts --report   print the health metrics

import { readdir, readFile, writeFile } from "node:fs/promises";
import { Command } from "./lib/command.ts";

export const LIMITS = {
  rustFile: 1500,
  tsFile: 800,
  function: 150,
  hooks: 40,
} as const;

export type Kind = "file" | "function" | "hooks";

export interface Measure {
  kind: Kind;
  /** `path` for files and hooks, `path::Qualified::name` for functions. */
  key: string;
  path: string;
  size: number;
  limit: number;
}

export interface Baseline {
  schema: 1;
  limits: typeof LIMITS;
  file: Record<string, number>;
  function: Record<string, number>;
  hooks: Record<string, number>;
}

const SOURCE_ROOTS = ["src/", "components/", "web/src/", "plugins/", "tools/"];
const BASELINE = "tools/structure-baseline.json";

/** Test drivers, fixtures and generated files are not production code. */
export function measured(path: string): boolean {
  if (!SOURCE_ROOTS.some((root) => path.startsWith(root))) return false;
  if (!/\.(rs|ts|tsx)$/.test(path) || path.endsWith(".d.ts")) return false;
  const name = path.slice(path.lastIndexOf("/") + 1);
  if (/(^|[._-])test(s)?([._-]|$)|Tests?\.|fixture|Conformance|conformance/.test(name)) {
    return false;
  }
  if (/\.generated\.ts$/.test(name)) return false;
  if (/(^|\/)(tests?|fixtures|vendor|node_modules|dist)\//.test(path)) {
    return false;
  }
  if (/(^|\/)[a-z_]*tests\//.test(path)) return false;
  return true;
}

/** The area an allowance may move within: a crate or package. */
export function area(path: string): string {
  const parts = path.split("/");
  if (parts[0] === "components" || parts[0] === "plugins") {
    return `${parts[0]}/${parts[1]}`;
  }
  if (parts[0] === "web") return "web";
  return parts[0];
}

// ---------------------------------------------------------------------------
// Lexing: replace comments and literal contents with spaces, keep newlines.

function blank(text: string): string {
  return text.replace(/[^\n]/g, " ");
}

function identChar(c: string | undefined): boolean {
  return c !== undefined && /[A-Za-z0-9_$]/.test(c);
}

/** Rust source with comments and string/char literal contents blanked. */
export function rustCode(source: string): string {
  let out = "";
  let i = 0;
  const n = source.length;
  while (i < n) {
    const c = source[i];
    const next = source[i + 1];
    if (c === "/" && next === "/") {
      const end = source.indexOf("\n", i);
      const stop = end === -1 ? n : end;
      out += blank(source.slice(i, stop));
      i = stop;
      continue;
    }
    if (c === "/" && next === "*") {
      let depth = 0;
      let j = i;
      while (j < n) {
        if (source[j] === "/" && source[j + 1] === "*") {
          depth++;
          j += 2;
        } else if (source[j] === "*" && source[j + 1] === "/") {
          depth--;
          j += 2;
          if (depth === 0) break;
        } else j++;
      }
      out += blank(source.slice(i, j));
      i = j;
      continue;
    }
    // Raw strings: r"..", r#".."#, br#".."#.
    const raw = /^b?r(#*)"/.exec(source.slice(i, i + 260));
    if (raw && !identChar(source[i - 1])) {
      const hashes = raw[1];
      const close = `"${hashes}`;
      const start = i + raw[0].length;
      const end = source.indexOf(close, start);
      const stop = end === -1 ? n : end + close.length;
      out += source.slice(i, start) + blank(source.slice(start, stop - close.length)) + close;
      i = stop;
      continue;
    }
    if (c === '"' || (c === "b" && next === '"' && !identChar(source[i - 1]))) {
      const open = c === "b" ? 2 : 1;
      let j = i + open;
      while (j < n && source[j] !== '"') j += source[j] === "\\" ? 2 : 1;
      out += source.slice(i, i + open) + blank(source.slice(i + open, j)) + '"';
      i = j + 1;
      continue;
    }
    if (c === "'") {
      // A char literal, not a lifetime: 'x', '\n', '\u{..}', or a surrogate pair.
      let end = -1;
      if (next === "\\") {
        end = source.indexOf("'", i + 2);
      } else if (source[i + 2] === "'") {
        end = i + 2;
      } else if (next !== undefined && /[\uD800-\uDBFF]/.test(next) && source[i + 3] === "'") {
        end = i + 3;
      }
      if (end !== -1 && end - i <= 12) {
        out += "'" + blank(source.slice(i + 1, end)) + "'";
        i = end + 1;
        continue;
      }
    }
    out += c;
    i++;
  }
  return out;
}

const REGEX_PREFIX =
  /[(,=:[!&|?{};+\-*%<>~^]$|(^|[^\w$])(return|typeof|case|do|else|in|of|void|yield|await|delete|instanceof|new)$/;

/** TypeScript/TSX source with comments and literal contents blanked. A quote
 * that does not close on its line is treated as text (JSX apostrophes). */
export function tsCode(source: string): string {
  let out = "";
  let i = 0;
  const n = source.length;
  // Each entry is the brace depth at which a template `${` expression opened.
  const templates: number[] = [];
  let depth = 0;
  const significant = (): string => out.replace(/\s+$/, "").slice(-12);
  const template = (): void => {
    // At the character after a backtick or a closing `}` of an expression.
    let j = i;
    while (j < n) {
      if (source[j] === "\\") {
        j += 2;
        continue;
      }
      if (source[j] === "`") {
        out += blank(source.slice(i, j)) + "`";
        i = j + 1;
        return;
      }
      if (source[j] === "$" && source[j + 1] === "{") {
        out += blank(source.slice(i, j)) + "${";
        i = j + 2;
        templates.push(depth);
        depth++;
        return;
      }
      j++;
    }
    out += blank(source.slice(i));
    i = n;
  };
  while (i < n) {
    const c = source[i];
    const next = source[i + 1];
    if (c === "/" && next === "/") {
      const end = source.indexOf("\n", i);
      const stop = end === -1 ? n : end;
      out += blank(source.slice(i, stop));
      i = stop;
      continue;
    }
    if (c === "/" && next === "*") {
      const end = source.indexOf("*/", i + 2);
      const stop = end === -1 ? n : end + 2;
      out += blank(source.slice(i, stop));
      i = stop;
      continue;
    }
    if (c === "'" || c === '"') {
      let j = i + 1;
      while (j < n && source[j] !== c && source[j] !== "\n") {
        j += source[j] === "\\" ? 2 : 1;
      }
      if (source[j] === c) {
        out += c + blank(source.slice(i + 1, j)) + c;
        i = j + 1;
        continue;
      }
      out += c;
      i++;
      continue;
    }
    if (c === "`") {
      out += "`";
      i++;
      template();
      continue;
    }
    if (c === "/" && REGEX_PREFIX.test(significant())) {
      let j = i + 1;
      let inClass = false;
      while (j < n && source[j] !== "\n") {
        if (source[j] === "\\") {
          j += 2;
          continue;
        }
        if (source[j] === "[") inClass = true;
        else if (source[j] === "]") inClass = false;
        else if (source[j] === "/" && !inClass) break;
        j++;
      }
      if (source[j] === "/") {
        out += "/" + blank(source.slice(i + 1, j)) + "/";
        i = j + 1;
        continue;
      }
    }
    if (c === "{") depth++;
    if (c === "}") {
      depth--;
      if (templates.length > 0 && templates[templates.length - 1] === depth) {
        templates.pop();
        out += "}";
        i++;
        template();
        continue;
      }
    }
    out += c;
    i++;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Structure.

function lineAt(code: string, index: number): number {
  let line = 1;
  for (let k = 0; k < index; k++) if (code.charCodeAt(k) === 10) line++;
  return line;
}

/** Index of the bracket closing the one at `open`, or -1. */
function matching(code: string, open: number): number {
  const opener = code[open];
  const closer = opener === "{" ? "}" : opener === "(" ? ")" : "]";
  let depth = 0;
  for (let k = open; k < code.length; k++) {
    if (code[k] === opener) depth++;
    else if (code[k] === closer) {
      depth--;
      if (depth === 0) return k;
    }
  }
  return -1;
}

const RUST_TEST_ATTRIBUTE = /#\[(?:cfg\((?:all\()?test\b[^\]]*|(?:tokio::)?test\b[^\]]*)\]/g;

/** Blank Rust items marked as test-only; returns the code and lines removed. */
export function stripRustTests(code: string): { code: string; removed: number } {
  let result = code;
  let removed = 0;
  RUST_TEST_ATTRIBUTE.lastIndex = 0;
  for (;;) {
    const match = RUST_TEST_ATTRIBUTE.exec(result);
    if (!match) break;
    // The item ends at its first top-level `;`, at the brace closing its body,
    // or — for a field, variant or match arm — at its `,` or before the `}`
    // closing the enclosing list.
    let end = -1;
    let k = match.index + match[0].length;
    let parens = 0;
    while (k < result.length) {
      const ch = result[k];
      if (ch === "(" || ch === "[") parens++;
      else if (ch === ")" || ch === "]") parens--;
      else if ((ch === ";" || ch === ",") && parens === 0) {
        end = k;
        break;
      } else if (ch === "}" && parens === 0) {
        end = k - 1;
        break;
      } else if (ch === "{" && parens === 0) {
        end = matching(result, k);
        break;
      }
      k++;
    }
    if (end === -1) break;
    const span = result.slice(match.index, end + 1);
    removed += (span.match(/\n/g) ?? []).length;
    result = result.slice(0, match.index) + blank(span) + result.slice(end + 1);
    RUST_TEST_ATTRIBUTE.lastIndex = match.index + 1;
  }
  return { code: result, removed };
}

interface Found {
  name: string;
  size: number;
}

/** Rust functions with bodies, qualified by enclosing impl/trait/mod names. */
export function rustFunctions(code: string): Found[] {
  const scopes: { name: string; end: number }[] = [];
  const found: Found[] = [];
  const token = /\b(impl|trait|mod|fn)\b/g;
  for (let match = token.exec(code); match; match = token.exec(code)) {
    while (scopes.length > 0 && scopes[scopes.length - 1].end < match.index) {
      scopes.pop();
    }
    const rest = code.slice(match.index + match[0].length);
    if (match[1] === "fn") {
      const name = /^\s+([A-Za-z_][A-Za-z0-9_]*)/.exec(rest);
      if (!name) continue;
      const params = code.indexOf("(", match.index);
      const close = params === -1 ? -1 : matching(code, params);
      if (close === -1) continue;
      let k = close + 1;
      while (k < code.length && code[k] !== "{" && code[k] !== ";") k++;
      if (code[k] !== "{") continue;
      const end = matching(code, k);
      if (end === -1) continue;
      const qualified = [...scopes.map((scope) => scope.name), name[1]].join("::");
      found.push({
        name: qualified,
        size: lineAt(code, end) - lineAt(code, match.index) + 1,
      });
      continue;
    }
    // impl/trait/mod opening a block.
    const header = /^([^;{]*)\{/.exec(rest);
    if (!header) continue;
    let label = header[1].trim();
    if (match[1] === "impl") {
      label = label.replace(/^<[^]*?>\s*/, "");
      const target = label.includes(" for ") ? label.slice(label.lastIndexOf(" for ") + 5) : label;
      label = target
        .replace(/<[^]*$/, "")
        .replace(/\bwhere\b[^]*$/, "")
        .trim();
      const separator = label.lastIndexOf("::");
      if (separator !== -1) label = label.slice(separator + 2);
    } else {
      label = label.split(/[\s<:]/)[0];
    }
    const open = match.index + match[0].length + header[0].length - 1;
    const end = matching(code, open);
    if (end === -1 || !label) continue;
    scopes.push({ name: label, end });
  }
  return found;
}

const TS_DECLARATIONS = [
  /\bfunction\s*\*?\s*([A-Za-z_$][\w$]*)\s*[<(]/g,
  /\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*(?::[^=;]+)?=\s*(?:async\s+)?(?:function\b|\(|[A-Za-z_$][\w$]*\s*=>|<[A-Za-z]|(?:React\.)?(?:memo|forwardRef)\s*\()/g,
  /^[ \t]*(?:(?:public|private|protected|static|async|override|readonly|get|set)\s+)*([A-Za-z_$][\w$]*)\s*(?:<[^>\n]*>)?\s*\([^;\n]*\)\s*(?::[^{;\n]+)?\{/gm,
];

const NOT_METHODS = new Set([
  "if",
  "for",
  "while",
  "switch",
  "catch",
  "with",
  "return",
  "function",
  "else",
]);

/** Named TypeScript functions, arrow functions, components and methods. */
export function tsFunctions(code: string): Found[] {
  const found = new Map<number, Found>();
  for (const pattern of TS_DECLARATIONS) {
    pattern.lastIndex = 0;
    for (let match = pattern.exec(code); match; match = pattern.exec(code)) {
      const name = match[1];
      if (NOT_METHODS.has(name)) continue;
      const start = match.index + match[0].length - 1;
      let body = -1;
      const arrow = code.indexOf("=>", start);
      const params = code.indexOf("(", match.index + match[0].indexOf(name) + name.length);
      if (pattern === TS_DECLARATIONS[1] && arrow !== -1) {
        // Arrow body: a block or a parenthesised expression right after `=>`.
        const after = /\S/.exec(code.slice(arrow + 2));
        const k = after ? arrow + 2 + after.index : -1;
        if (k !== -1 && (code[k] === "{" || code[k] === "(")) body = k;
        // memo(function X() {...}) and `= function` bodies.
        const fn = code.slice(match.index, arrow).match(/function\b[^{]*\{/);
        if (fn?.index !== undefined) {
          body = match.index + fn.index + fn[0].length - 1;
        }
      } else if (params !== -1) {
        const close = matching(code, params);
        if (close === -1) continue;
        let k = close + 1;
        while (k < code.length && code[k] !== "{" && code[k] !== ";") k++;
        if (code[k] === "{") body = k;
      }
      if (body === -1) continue;
      const end = matching(code, body);
      if (end === -1) continue;
      const lineStart = code.lastIndexOf("\n", match.index) + 1;
      const size = lineAt(code, end) - lineAt(code, match.index) + 1;
      const previous = found.get(lineStart);
      if (!previous || previous.size < size) {
        found.set(lineStart, { name, size });
      }
    }
  }
  return [...found.entries()].sort((a, b) => a[0] - b[0]).map(([, value]) => value);
}

export function hookCalls(code: string): number {
  return (code.match(/(?<![\w$.])(?:React\.)?use[A-Z][\w$]*\s*(?:<[^>()\n]*>)?\s*\(/g) ?? [])
    .length;
}

/** Measure one file; only over-limit measures are interesting to callers. */
export function measureFile(path: string, source: string): Measure[] {
  const measures: Measure[] = [];
  let code: string;
  let lines = source.split("\n").length - (source.endsWith("\n") ? 1 : 0);
  let functions: Found[];
  if (path.endsWith(".rs")) {
    const stripped = stripRustTests(rustCode(source));
    code = stripped.code;
    lines -= stripped.removed;
    functions = rustFunctions(code);
  } else {
    code = tsCode(source);
    functions = tsFunctions(code);
  }
  const fileLimit = path.endsWith(".rs") ? LIMITS.rustFile : LIMITS.tsFile;
  measures.push({
    kind: "file",
    key: path,
    path,
    size: lines,
    limit: fileLimit,
  });
  const seen = new Map<string, number>();
  for (const fn of functions) {
    const count = (seen.get(fn.name) ?? 0) + 1;
    seen.set(fn.name, count);
    const key = `${path}::${fn.name}${count > 1 ? `#${count}` : ""}`;
    measures.push({
      kind: "function",
      key,
      path,
      size: fn.size,
      limit: LIMITS.function,
    });
  }
  if (path.endsWith(".tsx")) {
    measures.push({
      kind: "hooks",
      key: path,
      path,
      size: hookCalls(code),
      limit: LIMITS.hooks,
    });
  }
  return measures;
}

// ---------------------------------------------------------------------------
// Ratchet.

export function emptyBaseline(): Baseline {
  return {
    schema: 1,
    limits: { ...LIMITS },
    file: {},
    function: {},
    hooks: {},
  };
}

export interface Verdict {
  failures: string[];
  /** Entities that may be lowered or dropped by `--update`. */
  lowerable: number;
}

export function check(measures: Measure[], baseline: Baseline): Verdict {
  const failures: string[] = [];
  let lowerable = 0;
  const present = new Set<string>();
  for (const measure of measures) {
    const recorded = baseline[measure.kind][measure.key];
    if (recorded !== undefined) present.add(`${measure.kind} ${measure.key}`);
    if (measure.size <= measure.limit) {
      if (recorded !== undefined) lowerable++;
      continue;
    }
    if (recorded === undefined) {
      failures.push(
        `new ${measure.kind} over its limit: ${measure.key} is ${measure.size} (limit ${measure.limit}); ` +
          "split it, or record a pure move with --update",
      );
    } else if (measure.size > recorded) {
      failures.push(
        `${measure.kind} grew past its baseline: ${measure.key} is ${measure.size} (recorded ${recorded}, limit ${measure.limit})`,
      );
    } else if (measure.size < recorded) {
      lowerable++;
    }
  }
  for (const kind of ["file", "function", "hooks"] as const) {
    for (const key of Object.keys(baseline[kind])) {
      if (!present.has(`${kind} ${key}`)) lowerable++;
    }
  }
  return { failures, lowerable };
}

/** Lower recorded sizes and admit new over-limit entities while each area's
 * recorded total per kind does not grow beyond a formatting slack. */
export function update(
  measures: Measure[],
  baseline: Baseline,
): { baseline: Baseline; refused: string[] } {
  const next = emptyBaseline();
  const refused: string[] = [];
  const budget = new Map<string, number>();
  const used = new Map<string, number>();
  for (const kind of ["file", "function", "hooks"] as const) {
    for (const [key, size] of Object.entries(baseline[kind])) {
      const slot = `${kind} ${area(key)}`;
      budget.set(slot, (budget.get(slot) ?? 0) + size);
    }
  }
  const fresh: Measure[] = [];
  for (const measure of measures) {
    if (measure.size <= measure.limit) continue;
    const recorded = baseline[measure.kind][measure.key];
    if (recorded === undefined) {
      fresh.push(measure);
      continue;
    }
    const size = Math.min(recorded, measure.size);
    next[measure.kind][measure.key] = size;
    const slot = `${measure.kind} ${area(measure.path)}`;
    used.set(slot, (used.get(slot) ?? 0) + size);
    if (measure.size > recorded) {
      refused.push(`${measure.key} grew to ${measure.size}; --update never raises a recorded size`);
    }
  }
  for (const measure of fresh.sort((a, b) => a.size - b.size)) {
    const slot = `${measure.kind} ${area(measure.path)}`;
    const total = (used.get(slot) ?? 0) + measure.size;
    // A pure move adds a module header and rustfmt re-wraps signatures that
    // gained a visibility. Allow 1% of the recorded total for that, always
    // below one limit, so the slack alone can never admit a new offender.
    const recorded = budget.get(slot) ?? 0;
    const slack = Math.min(Math.floor(recorded / 100), measure.limit - 1);
    if (total <= recorded + slack) {
      next[measure.kind][measure.key] = measure.size;
      used.set(slot, total);
    } else {
      refused.push(
        `${measure.kind} ${measure.key} (${measure.size}) is new and ${area(
          measure.path,
        )} has no freed allowance; split it below ${measure.limit}`,
      );
    }
  }
  return { baseline: next, refused };
}

export function report(measures: Measure[]): string[] {
  const files = measures.filter((m) => m.kind === "file");
  const functions = measures.filter((m) => m.kind === "function");
  return [
    `production files measured: ${files.length}`,
    `files over 3000 lines: ${files.filter((m) => m.size > 3000).length}`,
    `files over their limit: ${files.filter((m) => m.size > m.limit).length}`,
    `functions measured: ${functions.length}`,
    ...[150, 200, 400, 800].map(
      (n) => `functions over ${n} lines: ${functions.filter((m) => m.size > n).length}`,
    ),
    `TSX files over ${LIMITS.hooks} hook calls: ${
      measures.filter((m) => m.kind === "hooks" && m.size > m.limit).length
    }`,
    "largest functions:",
    ...functions
      .sort((a, b) => b.size - a.size)
      .slice(0, 10)
      .map((m) => `  ${m.size}  ${m.key}`),
  ];
}

// ---------------------------------------------------------------------------
// CLI.

async function repositorySources(): Promise<string[]> {
  let listed: Uint8Array | null = null;
  try {
    const output = await new Command("git", {
      args: ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
      stdout: "piped",
      stderr: "null",
    }).output();
    if (output.success) listed = output.stdout;
  } catch {
    // No git executable (a Nix build): fall through to the tree walk.
  }
  if (listed !== null) {
    return new TextDecoder().decode(listed).split("\0").filter(measured).sort();
  }
  // A Nix build source has no .git and holds exactly the tracked files, plus
  // the dependency trees the build installed; walk it instead.
  const found: string[] = [];
  const walk = async (dir: string): Promise<void> => {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      if (entry.name === "node_modules" || entry.name === "dist" || entry.name === "target")
        continue;
      const path = dir === "." ? entry.name : `${dir}/${entry.name}`;
      if (entry.isDirectory()) await walk(path);
      else if (entry.isFile()) found.push(path);
    }
  };
  await walk(".");
  return found.filter(measured).sort();
}

export function serialize(baseline: Baseline): string {
  const sorted = (record: Record<string, number>) =>
    Object.fromEntries(Object.entries(record).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
  return `${JSON.stringify(
    {
      schema: 1,
      limits: baseline.limits,
      file: sorted(baseline.file),
      function: sorted(baseline.function),
      hooks: sorted(baseline.hooks),
    },
    null,
    2,
  )}\n`;
}

async function main(argv: string[]): Promise<number> {
  const measures: Measure[] = [];
  for (const path of await repositorySources()) {
    measures.push(...measureFile(path, await readFile(path, "utf8")));
  }
  if (argv.includes("--report")) {
    for (const line of report(measures)) console.log(line);
    return 0;
  }
  let baseline = emptyBaseline();
  try {
    baseline = JSON.parse(await readFile(BASELINE, "utf8")) as Baseline;
  } catch (error) {
    if (!argv.includes("--init")) throw error;
  }
  if (argv.includes("--init")) {
    // One-time seeding: record every current offender as-is.
    const seeded = emptyBaseline();
    for (const measure of measures) {
      if (measure.size > measure.limit) {
        seeded[measure.kind][measure.key] = measure.size;
      }
    }
    await writeFile(BASELINE, serialize(seeded));
    return 0;
  }
  if (JSON.stringify(baseline.limits) !== JSON.stringify(LIMITS)) {
    console.error(
      `${BASELINE} records different limits; limits change only together with the baseline`,
    );
    return 1;
  }
  if (argv.includes("--update")) {
    const { baseline: next, refused } = update(measures, baseline);
    for (const line of refused) console.error(line);
    if (refused.length > 0) return 1;
    await writeFile(BASELINE, serialize(next));
    return 0;
  }
  const verdict = check(measures, baseline);
  for (const line of verdict.failures) console.error(line);
  if (verdict.failures.length > 0) return 1;
  if (verdict.lowerable > 0) {
    console.log(`structure ok; ${verdict.lowerable} baseline entries can be lowered with --update`);
  } else {
    console.log("structure ok");
  }
  return 0;
}

if (import.meta.main) {
  process.exit(await main(process.argv.slice(2)));
}
