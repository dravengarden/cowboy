// Test-only inputs. The checked-in fixture is not runtime authorization data.
import type { CompositionCheckCode } from "./composition-check.ts";

interface Edit {
  readonly path: string;
  readonly value?: unknown;
  readonly remove?: boolean;
}
interface Vector {
  readonly name: string;
  readonly raw?: string;
  readonly edits?: readonly Edit[];
  readonly decode: boolean;
  readonly error?: CompositionCheckCode;
}
const fixture = JSON.parse(
  await Deno.readTextFile(
    new URL("../tests/fixtures/composition-v1.json", import.meta.url),
  ),
) as { base: unknown; vectors: Vector[] };

function at(value: unknown, parts: readonly string[]): unknown {
  for (const part of parts) {
    if (
      value === null || typeof value !== "object" || !Object.hasOwn(value, part)
    ) {
      throw new Error("invalid fixture path");
    }
    value = Reflect.get(value, part);
  }
  return value;
}

function patch(value: unknown, edits: readonly Edit[]): unknown {
  const result: unknown = structuredClone(value);
  for (const edit of edits) {
    const parts = edit.path.slice(1).split("/");
    const key = parts.pop()!;
    const parent = at(result, parts);
    if (parent === null || typeof parent !== "object") {
      throw new Error("invalid fixture parent");
    }
    if (edit.remove) Reflect.deleteProperty(parent, key);
    else {
      Object.defineProperty(parent, key, {
        value: structuredClone(edit.value),
        configurable: true,
        writable: true,
        enumerable: true,
      });
    }
  }
  return result;
}

export function baseInput(edits: readonly Edit[] = []): string {
  return JSON.stringify(patch(fixture.base, edits));
}

export interface LinkVector {
  readonly name: string;
  readonly raw: string;
  readonly decode: boolean;
  readonly error?: CompositionCheckCode;
}

/** Also exercised against the actual Rust CLI, not just the TS implementation. */
export function linkVectors(): LinkVector[] {
  const vectors: LinkVector[] = fixture.vectors.map((vector) => ({
    name: vector.name,
    raw: vector.raw ?? baseInput(vector.edits),
    decode: vector.decode,
    error: vector.error,
  }));
  const extra = (
    name: string,
    edits: Edit[],
    error?: CompositionCheckCode,
  ): void => {
    vectors.push({
      name,
      raw: baseInput(edits),
      decode: error !== "invalid_contract",
      error,
    });
  };
  const node = at(fixture.base, ["nodes", "1"]);
  const peer = patch(node, [{ path: "/id", value: "victoria-new" }]);
  const otherVersion = patch(peer, [{
    path: "/identity/release/version",
    value: "2.0.0",
  }]);
  extra("one generation refuses two exact versions", [{
    path: "/nodes/2",
    value: otherVersion,
  }], "generation_conflict");
  extra("distinct generations retain distinct versions", [{
    path: "/nodes/2",
    value: patch(otherVersion, [{ path: "/identity/generation", value: "2" }]),
  }]);
  for (
    const [field, value] of [
      ["digest", `sha256:${"f".repeat(64)}`],
      ["publisher", "different-publisher"],
    ]
  ) {
    extra(`one generation refuses changed ${field}`, [{
      path: "/nodes/2",
      value: patch(peer, [{ path: `/identity/release/${field}`, value }]),
    }], "generation_conflict");
  }
  extra("two incarnations may share one exact release", [{
    path: "/nodes/2",
    value: patch(peer, [{ path: "/identity/incarnation", value: "2" }]),
  }]);
  extra("distinct Machine installation slots may differ", [{
    path: "/nodes/2",
    value: patch(otherVersion, [{ path: "/site/machine_id", value: "falcon" }]),
  }]);
  const secondBinding = patch(at(fixture.base, ["bindings", "0"]), [{
    path: "/provider/node",
    value: "victoria-new",
  }]);
  for (const cardinality of ["one", "optional", "many"]) {
    extra(`two providers with ${cardinality} cardinality`, [
      { path: "/nodes/2", value: peer },
      { path: "/bindings/1", value: secondBinding },
      { path: "/nodes/0/requires/0/cardinality", value: cardinality },
    ], cardinality === "many" ? undefined : "cardinality_mismatch");
  }
  extra("missing node owner", [{
    path: "/nodes/1/owner",
    value: { kind: "node", node: "missing" },
  }], "invalid_owner");
  extra("self ownership", [{
    path: "/nodes/1/owner",
    value: { kind: "node", node: "victoria" },
  }], "invalid_owner");
  const port = at(node, ["provides", "0"]);
  const ports = Array.from(
    { length: 32 },
    (_, index) => patch(port, [{ path: "/id", value: `port-${index}` }]),
  );
  const nodeBudget = Array.from({ length: 65 }, (_, index) =>
    patch(node, [
      { path: "/id", value: `node-${index}` },
      { path: "/provides", value: ports },
    ]));
  extra("total port budget is global", [
    { path: "/nodes", value: nodeBudget },
    { path: "/bindings", value: [] },
  ], "port_budget");
  const id = (index: number): string =>
    `node-${String(index).padStart(3, "0")}`;
  const chain = Array.from({ length: 256 }, (_, index) =>
    patch(node, [
      { path: "/id", value: id(index) },
      { path: "/provides", value: [] },
      {
        path: "/owner",
        value: index === 0
          ? { kind: "scope" }
          : { kind: "node", node: id(index - 1) },
      },
    ]));
  extra("maximum bounded ownership chain", [{ path: "/nodes", value: chain }, {
    path: "/bindings",
    value: [],
  }]);
  extra("reversed declarations preserve the maximum chain", [{
    path: "/nodes",
    value: [...chain].reverse(),
  }, { path: "/bindings", value: [] }]);
  extra("maximum ownership cycle fails closed", [
    { path: "/nodes", value: chain },
    { path: "/bindings", value: [] },
    { path: "/nodes/0/owner", value: { kind: "node", node: id(255) } },
  ], "dependency_cycle");
  extra(
    "node count budget",
    [{ path: "/nodes", value: [...chain, node] }],
    "invalid_contract",
  );
  // Locale-sensitive punctuation must not alter Rust's exact ASCII ordering.
  const punctuation = ["a-a", "a.a", "a_a", "aa"].map((name) =>
    patch(node, [
      { path: "/id", value: name },
      { path: "/provides", value: [] },
    ])
  );
  extra("ASCII ordering is independent of the browser locale", [
    { path: "/nodes", value: punctuation.reverse() },
    { path: "/bindings", value: [] },
  ]);
  vectors.push({
    name: "byte budget includes trailing whitespace",
    raw: `${baseInput()}${" ".repeat(1048576)}`,
    decode: false,
    error: "invalid_json",
  });
  const reverseKeys = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(reverseKeys);
    if (value !== null && typeof value === "object") {
      return Object.fromEntries(
        Object.entries(value).reverse().map(
          ([key, item]) => [key, reverseKeys(item)],
        ),
      );
    }
    return value;
  };
  // Serde reconstructs wire-record order. The browser must produce the same
  // canonical digest AND projection when an author reverses every object key.
  vectors.push(
    ...vectors.filter((vector) => vector.error === undefined).map(
      (vector) => ({
        name: `${vector.name} / reversed JSON properties`,
        raw: JSON.stringify(reverseKeys(JSON.parse(vector.raw))),
        decode: true,
      }),
    ),
  );
  return vectors;
}
