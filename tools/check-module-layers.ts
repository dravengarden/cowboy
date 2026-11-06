// Module layering ratchet (docs/structural-refactor-plan-2026-11.md, slice 0.5).
//
// Builds the dependency graph between the top-level modules of each Rust
// crate in the workspace from production code (test items and test files
// excluded) and compares it with tools/module-edges-baseline.json:
//
//   - no module may join a dependency cycle it is not already part of, and no
//     two cycles may merge: every current cycle must fit inside a recorded one;
//   - no new edge may be added between two modules of the same cycle, because
//     that ties the knot tighter even when the membership is unchanged;
//   - any other new edge is allowed, and edges may disappear freely.
//
// References are resolved through `crate::` paths (including grouped
// `use crate::{a, b::c}` trees and aliases), `super::` chains that climb to
// the crate root, direct paths into sibling workspace crates, and the
// `pub use other::*` / `pub use other::module` re-exports in each lib.rs, so
// changing import syntax cannot hide an edge. `--update` rewrites the
// baseline after a passing check (it can only shrink cycles); `--report`
// prints the cycles.

import { readdir, readFile, writeFile } from "node:fs/promises";
import { join, relative } from "node:path";
import { rustCode, stripRustTests } from "./check-structure.ts";

export const CRATES: Record<string, string> = {
  cowboy: "src",
  cowboy_server: "components/server/src",
  cowboy_machine: "components/machine/src",
  cowboy_wire: "components/wire/src",
  cowboy_plugin_sdk: "components/plugin-sdk/src",
  cowboy_provider_sdk: "components/provider-sdk/src",
};

const BASELINE = "tools/module-edges-baseline.json";
const ROOT_FILES = new Set(["lib.rs", "main.rs", "bin"]);

export interface CrateSource {
  name: string;
  /** Top-level module name -> production files (path relative to repo, code). */
  modules: Map<string, { path: string; code: string }[]>;
  /** lib.rs code with tests stripped, for re-export resolution. */
  root: string;
}

export interface Baseline {
  schema: 1;
  cycles: string[][];
  edges: string[];
}

function testFile(name: string): boolean {
  return /(^|[._-])tests?([._-]|$)|conformance|fixture/i.test(name);
}

async function rustFiles(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (testFile(entry.name)) continue;
      out.push(...(await rustFiles(path)));
    } else if (entry.name.endsWith(".rs") && !testFile(entry.name)) {
      out.push(path);
    }
  }
  return out;
}

export async function loadCrate(repo: string, name: string, dir: string): Promise<CrateSource> {
  const modules = new Map<string, { path: string; code: string }[]>();
  const base = join(repo, dir);
  for (const entry of await readdir(base, { withFileTypes: true })) {
    if (ROOT_FILES.has(entry.name) || testFile(entry.name)) continue;
    const module = entry.name.replace(/\.rs$/, "");
    const files = entry.isDirectory()
      ? await rustFiles(join(base, entry.name))
      : entry.name.endsWith(".rs")
        ? [join(base, entry.name)]
        : [];
    for (const file of files) {
      const code = stripRustTests(rustCode(await readFile(file, "utf8"))).code;
      const list = modules.get(module) ?? [];
      list.push({ path: relative(repo, file), code });
      modules.set(module, list);
    }
  }
  let root = "";
  try {
    root = stripRustTests(rustCode(await readFile(join(base, "lib.rs"), "utf8"))).code;
  } catch {
    // A binary-only crate has no library surface to re-export from.
  }
  return { name, modules, root };
}

/** Leading identifiers of a `{a, b::c, d::{e}}` use tree starting at `open`. */
function groupHeads(code: string, open: number): string[] {
  const heads: string[] = [];
  let depth = 0;
  let start = open + 1;
  for (let k = open; k < code.length; k++) {
    const ch = code[k];
    if (ch === "{") depth++;
    else if (ch === "}") depth--;
    if ((ch === "," && depth === 1) || depth === 0) {
      const head = /^\s*([A-Za-z_][A-Za-z0-9_]*)/.exec(code.slice(start, k));
      if (head && head[1] !== "self") heads.push(head[1]);
      start = k + 1;
      if (depth === 0) break;
    }
  }
  return heads;
}

/** Names a file refers to at its crate root, and paths into other crates. */
export function references(
  code: string,
  depth: number,
  crates: string[],
): { local: Set<string>; external: Set<string> } {
  const local = new Set<string>();
  const external = new Set<string>();
  const ident = "[A-Za-z_][A-Za-z0-9_]*";
  for (const match of code.matchAll(new RegExp(`(?<![\\w$:])crate::(?:(${ident})|\\{)`, "g"))) {
    if (match[1]) local.add(match[1]);
    else {
      for (const head of groupHeads(code, match.index + match[0].length - 1)) local.add(head);
    }
  }
  // `super::` repeated `depth` times or more climbs to the crate root.
  for (const match of code.matchAll(
    new RegExp(`(?<![\\w$:])((?:super::)+)(?:(${ident})|\\{)`, "g"),
  )) {
    const climbs = match[1].length / "super::".length;
    if (climbs < depth) continue;
    if (match[2]) {
      if (match[2] !== "super") local.add(match[2]);
    } else {
      for (const head of groupHeads(code, match.index + match[0].length - 1)) local.add(head);
    }
  }
  for (const crate of crates) {
    for (const match of code.matchAll(
      new RegExp(`(?<![\\w$:])${crate}::(?:(${ident})|\\{)`, "g"),
    )) {
      if (match[1]) external.add(`${crate}::${match[1]}`);
      else {
        for (const head of groupHeads(code, match.index + match[0].length - 1))
          external.add(`${crate}::${head}`);
      }
    }
  }
  return { local, external };
}

/** Resolve `name` at the root of `crate` to a module node, following lib.rs
 * re-exports. Items that are not modules resolve to nothing. */
export function resolve(
  crates: Map<string, CrateSource>,
  crate: string,
  name: string,
  seen = new Set<string>(),
): string | null {
  const key = `${crate}::${name}`;
  if (seen.has(key)) return null;
  seen.add(key);
  const source = crates.get(crate);
  if (!source) return null;
  if (source.modules.has(name)) return key;
  const explicit = new RegExp(
    `pub\\s+use\\s+([A-Za-z_][A-Za-z0-9_]*)::([A-Za-z_][A-Za-z0-9_]*)(?:\\s+as\\s+([A-Za-z_][A-Za-z0-9_]*))?\\s*;`,
    "g",
  );
  for (const match of source.root.matchAll(explicit)) {
    if ((match[3] ?? match[2]) === name && crates.has(match[1])) {
      return resolve(crates, match[1], match[2], seen);
    }
  }
  for (const match of source.root.matchAll(/pub\s+use\s+([A-Za-z_][A-Za-z0-9_]*)::\*\s*;/g)) {
    if (!crates.has(match[1])) continue;
    const found = resolve(crates, match[1], name, seen);
    if (found) return found;
  }
  return null;
}

export function buildGraph(crates: Map<string, CrateSource>): Map<string, Set<string>> {
  const graph = new Map<string, Set<string>>();
  const names = [...crates.keys()];
  for (const [crate, source] of crates) {
    for (const [module, files] of source.modules) {
      const node = `${crate}::${module}`;
      const out = graph.get(node) ?? new Set<string>();
      graph.set(node, out);
      for (const file of files) {
        const inside = file.path.slice(file.path.indexOf(module));
        const depth = inside.endsWith("/mod.rs")
          ? inside.split("/").length - 1
          : inside.split("/").length;
        const { local, external } = references(
          file.code,
          depth,
          names.filter((name) => name !== crate),
        );
        for (const name of local) {
          const target = resolve(crates, crate, name);
          if (target && target !== node) out.add(target);
        }
        for (const path of external) {
          const [other, name] = path.split("::");
          const target = resolve(crates, other, name);
          if (target && target !== node) out.add(target);
        }
      }
    }
  }
  return graph;
}

/** Strongly connected components with more than one node (Tarjan). */
export function cycles(graph: Map<string, Set<string>>): string[][] {
  let counter = 0;
  const index = new Map<string, number>();
  const low = new Map<string, number>();
  const stack: string[] = [];
  const onStack = new Set<string>();
  const found: string[][] = [];
  const visit = (node: string): void => {
    index.set(node, counter);
    low.set(node, counter);
    counter++;
    stack.push(node);
    onStack.add(node);
    for (const next of graph.get(node) ?? []) {
      if (!index.has(next)) {
        visit(next);
        low.set(node, Math.min(low.get(node)!, low.get(next)!));
      } else if (onStack.has(next)) {
        low.set(node, Math.min(low.get(node)!, index.get(next)!));
      }
    }
    if (low.get(node) === index.get(node)) {
      const component: string[] = [];
      for (;;) {
        const top = stack.pop()!;
        onStack.delete(top);
        component.push(top);
        if (top === node) break;
      }
      if (component.length > 1) found.push(component.sort());
    }
  };
  for (const node of [...graph.keys()].sort()) {
    if (!index.has(node)) visit(node);
  }
  return found.sort((a, b) => b.length - a.length || (a[0] < b[0] ? -1 : 1));
}

export function edgeList(graph: Map<string, Set<string>>): string[] {
  const edges: string[] = [];
  for (const [from, targets] of graph) {
    for (const to of targets) edges.push(`${from} -> ${to}`);
  }
  return edges.sort();
}

export function check(graph: Map<string, Set<string>>, baseline: Baseline): string[] {
  const failures: string[] = [];
  const recordedEdges = new Set(baseline.edges);
  const current = cycles(graph);
  for (const cycle of current) {
    const home = baseline.cycles.find((recorded) => cycle.every((node) => recorded.includes(node)));
    if (!home) {
      const joined = cycle.filter(
        (node) => !baseline.cycles.some((recorded) => recorded.includes(node)),
      );
      failures.push(
        `new or enlarged dependency cycle (${cycle.length} modules): ${cycle.join(", ")}` +
          (joined.length > 0
            ? `; newly entangled: ${joined.join(", ")}`
            : "; it merges recorded cycles"),
      );
      continue;
    }
    const members = new Set(cycle);
    for (const from of cycle) {
      for (const to of graph.get(from) ?? []) {
        if (members.has(to) && !recordedEdges.has(`${from} -> ${to}`)) {
          failures.push(
            `new edge inside a dependency cycle: ${from} -> ${to}; depend on a narrower module or a shared contract instead`,
          );
        }
      }
    }
  }
  return failures;
}

export function snapshot(graph: Map<string, Set<string>>): Baseline {
  return { schema: 1, cycles: cycles(graph), edges: edgeList(graph) };
}

async function main(argv: string[]): Promise<number> {
  const crates = new Map<string, CrateSource>();
  for (const [name, dir] of Object.entries(CRATES)) {
    crates.set(name, await loadCrate(process.cwd(), name, dir));
  }
  const graph = buildGraph(crates);
  if (argv.includes("--report")) {
    for (const cycle of cycles(graph)) {
      console.log(`cycle of ${cycle.length}: ${cycle.join(", ")}`);
    }
    console.log(`edges: ${edgeList(graph).length}`);
    return 0;
  }
  if (argv.includes("--init")) {
    await writeFile(BASELINE, `${JSON.stringify(snapshot(graph), null, 2)}\n`);
    return 0;
  }
  const baseline = JSON.parse(await readFile(BASELINE, "utf8")) as Baseline;
  const failures = check(graph, baseline);
  for (const failure of failures) console.error(failure);
  if (failures.length > 0) return 1;
  if (argv.includes("--update")) {
    await writeFile(BASELINE, `${JSON.stringify(snapshot(graph), null, 2)}\n`);
    return 0;
  }
  const largest = cycles(graph)[0]?.length ?? 0;
  console.log(`module layers ok; largest cycle ${largest} modules`);
  return 0;
}

if (import.meta.main) {
  process.exit(await main(process.argv.slice(2)));
}
