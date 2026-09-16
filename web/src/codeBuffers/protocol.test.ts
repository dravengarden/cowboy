import { assert, assertEquals, assertThrows } from "jsr:@std/assert";
import golden from "../../../contracts/code-buffer-client.fixture.json" with {
  type: "json",
};
import { decodeObservation } from "./observations.ts";
import {
  BufferClientError,
  decodeResourceId,
  decodeSnapshot,
} from "./protocol.ts";
import { ID, OTHER, readWire, wire } from "./fixture.ts";

Deno.test("browser accepts the exact nonempty Rust HTTP fixture and recursively freezes observations", () => {
  assertEquals(decodeSnapshot(golden.prepared, 200), golden.prepared);
  const id = decodeResourceId(ID);
  const language = decodeObservation(golden.language, id, "language");
  const symbols = decodeObservation(golden.symbols, id, "symbols");
  assertEquals<unknown>(language, golden.language);
  assertEquals<unknown>(symbols, golden.symbols);
  assert(Object.isFrozen(language.result.diagnostics[0]!.start));
  assert(Object.isFrozen(symbols.result.symbols[0]!.children));
  assert(Object.isFrozen(language.openedVersion[0]));
});

Deno.test("snapshot codec rejects shape, identity, version and status/pending substitutions", () => {
  const cases: [unknown, number][] = [
    [wire("open"), 202],
    [wire("open", ID, true), 200],
    [wire("released", ID, true), 202],
    [wire("open", OTHER), 200],
    [{ ...wire("open"), apiVersion: 2 }, 200],
    [{ ...wire("open"), native: {} }, 200],
    [{ ...wire("open"), pending: 1 }, 200],
    [{ ...wire("open"), state: "done" }, 200],
    [
      { ...wire("open"), resourceId: "a".repeat(32) + "-0000000000000000" },
      200,
    ],
    [[], 200],
    [null, 200],
  ];
  for (const [value, status] of cases) {
    assertThrows(
      () => decodeSnapshot(value, status, decodeResourceId(ID)),
      BufferClientError,
    );
  }
});

Deno.test("language observations enforce exact tags, owners, bounded vectors and all nested fields", () => {
  const reject = (mutate: (value: typeof golden.language) => void) => {
    const value = structuredClone(golden.language);
    mutate(value);
    assertThrows(
      () => decodeObservation(value, decodeResourceId(ID), "language"),
      BufferClientError,
    );
  };
  reject((value) => {
    value.resourceId = OTHER;
  });
  reject((value) => {
    value.apiVersion = 2;
  });
  reject((value) => {
    value.result.kind = "symbols";
  });
  reject((value) => {
    value.result.diagnosticsState = "unobserved";
  });
  reject((value) => {
    Object.assign(value.result, { native: "private" });
  });
  reject((value) => {
    Object.assign(value.result.diagnostics[0]!, { code: "unknown" });
  });
  reject((value) => {
    value.result.diagnostics[0]!.start.column = -1;
  });
  reject((value) => {
    value.result.diagnostics[0]!.end.column = 0;
  });
  reject((value) => {
    value.result.diagnostics[0]!.severity = 2 ** 32;
  });
  reject((value) => {
    value.result.diagnostics[0]!.message = "界".repeat(30_000);
  });
  reject((value) => {
    value.result.inlayHints[0]!.offset = 2 ** 32;
  });
  reject((value) => {
    value.result.semanticTokens.push(1);
  });
  reject((value) => {
    value.result.semanticTokens[0] = 0.5;
  });
  reject((value) => {
    value.openedVersion.push(value.openedVersion[0]!);
  });
  reject((value) => {
    value.openedVersion[0]!.timestamp = NaN;
  });
  reject((value) => {
    value.openedVersion = Array.from(
      { length: 257 },
      (_, replicaId) => ({ replicaId, timestamp: 1 }),
    );
  });
  reject((value) => {
    value.result.diagnostics = Array.from(
      { length: 1001 },
      () => value.result.diagnostics[0]!,
    );
  });
  reject((value) => {
    value.result.inlayHints = Array.from(
      { length: 2001 },
      () => value.result.inlayHints[0]!,
    );
  });
  reject((value) => {
    value.result.semanticTokens = Array(50_005).fill(0);
  });
  assertThrows(
    () =>
      decodeObservation(readWire("language"), decodeResourceId(ID), "symbols"),
    BufferClientError,
  );
});

Deno.test("symbol observations bound total descendants, depth, integer kinds and selection ranges", () => {
  const symbol = golden.symbols.result.symbols[0]!;
  type Node = Omit<typeof symbol, "children"> & { children: Node[] };
  const node = (): Node => structuredClone(symbol);
  const reject = (symbols: Node[]) => {
    assertThrows(
      () =>
        decodeObservation(
          { ...golden.symbols, result: { kind: "symbols", symbols } },
          decodeResourceId(ID),
          "symbols",
        ),
      BufferClientError,
    );
  };
  const depth = node();
  let tail = depth;
  for (let index = 0; index < 16; index++) {
    const child = node();
    tail.children.push(child);
    tail = child;
  }
  reject([depth]);
  const broad = node();
  broad.children = Array.from({ length: 2_000 }, node);
  reject([broad]);
  const range = node();
  range.selectionEnd.row = 3;
  reject([range]);
  const fractional = node();
  fractional.kind = 0.5;
  reject([fractional]);
  const extra = node();
  Object.assign(extra.children, { unused: true });
  Object.assign(extra, { path: "not owned" });
  reject([extra]);
});
