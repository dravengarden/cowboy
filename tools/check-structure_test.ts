import { expect, test } from "bun:test";
import {
  check,
  emptyBaseline,
  hookCalls,
  LIMITS,
  type Measure,
  measured,
  measureFile,
  rustCode,
  rustFunctions,
  stripRustTests,
  tsCode,
  tsFunctions,
  update,
} from "./check-structure.ts";

const body = (lines: number, indent = "    "): string =>
  Array.from({ length: lines }, (_, i) => `${indent}let v${i} = ${i};`).join(
    "\n",
  );

test("production paths exclude tests, fixtures, conformance drivers and generated files", () => {
  expect(measured("src/core.rs")).toBe(true);
  expect(measured("web/src/App.tsx")).toBe(true);
  expect(measured("components/server/src/server.rs")).toBe(true);
  for (
    const path of [
      "src/core_tests.rs",
      "src/store/plugin_operations/tests/resolution.rs",
      "web/src/store.test.ts",
      "tools/check-structure_test.ts",
      "web/src/managedCallsBrowserConformance.tsx",
      "contracts/composition.generated.ts",
      "web/src/protocol.generated.ts",
      "components/server/src/server/agent_api/tests.rs",
      "components/machine/src/worker_execution_tests/native_managed.rs",
      "docs/notes.md",
      "web/src/types.d.ts",
    ]
  ) expect(measured(path)).toBe(false);
});

test("rust lexing ignores braces in comments, strings, raw strings and chars but keeps lifetimes", () => {
  const source = [
    "fn a<'x>(v: &'x str) -> char {",
    "    // }}}",
    "    /* { /* nested } */ { */",
    '    let s = "}{";',
    '    let r = r#"}"}"#;',
    "    let c = '}';",
    "    let e = '\\'';",
    "    '{'",
    "}",
  ].join("\n");
  const code = rustCode(source);
  expect(code.split("\n").length).toBe(9);
  expect((code.match(/\{/g) ?? []).length).toBe(1);
  expect((code.match(/\}/g) ?? []).length).toBe(1);
  expect(code).toContain("<'x>");
  expect(rustFunctions(code)).toEqual([{ name: "a", size: 9 }]);
});

test("rust test items are removed from production lines and functions", () => {
  const source = [
    "fn live() {",
    "}",
    "#[cfg(test)]",
    "mod tests {",
    "    #[test]",
    "    fn big() {",
    body(200, "        "),
    "    }",
    "}",
    "#[cfg(test)]",
    "use std::fmt;",
    "#[cfg_attr(not(test), allow(dead_code))]",
    "fn kept() {}",
    "struct S {",
    "    #[cfg(test)]",
    "    probe: Arc<AtomicU64>,",
    "    live: u8,",
    "}",
    "fn after() {",
    "    let s = S {",
    "        #[cfg(test)]",
    "        probe: Arc::new(AtomicU64::new(0)),",
    "        live: 1,",
    "    };",
    "}",
  ].join("\n");
  const stripped = stripRustTests(rustCode(source));
  expect(rustFunctions(stripped.code)).toEqual([
    { name: "live", size: 2 },
    { name: "kept", size: 1 },
    { name: "after", size: 7 },
  ]);
  const [file] = measureFile("src/x.rs", `${source}\n`);
  expect(stripped.removed).toBe(208);
  expect(file.size).toBe(25 + 199 - 208);
});

test("rust functions are qualified by impl, trait impl and mod; bodiless signatures are skipped", () => {
  const code = rustCode([
    "mod outer {",
    "    impl<T: Clone> Hub<T> where T: Send {",
    "        pub async fn push(&self) -> Result<(), Error> { Ok(()) }",
    "    }",
    "    impl std::fmt::Display for crate::core::Status {",
    "        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {",
    "            Ok(())",
    "        }",
    "    }",
    "    trait Sink { fn write(&self); fn flush(&self) {} }",
    "}",
  ].join("\n"));
  expect(rustFunctions(code).map((f) => f.name)).toEqual([
    "outer::Hub::push",
    "outer::Status::fmt",
    "outer::Sink::flush",
  ]);
});

test("ts lexing handles templates with expressions, regexes and JSX apostrophes", () => {
  const source = [
    "export function View(props: { a: string }) {",
    "  const t = `}${props.a.replace(/[}{]/g, \"{\")}${`${'}'}`}{`;",
    "  const r = /\\}{2}/;",
    "  const half = 1 / 2; const q = 3 / 4;",
    "  return <p>Don't {props.a} it's</p>;",
    "}",
  ].join("\n");
  const code = tsCode(source);
  expect(code.split("\n").length).toBe(6);
  expect(tsFunctions(code)).toEqual([{ name: "View", size: 6 }]);
});

test("ts functions: declarations, arrows, memo components, expression bodies and methods", () => {
  const code = tsCode([
    "export const A = memo(function A({ x }: Props) {",
    "  return x;",
    "});",
    "const B = async (a: number): Promise<void> => {",
    "  await a;",
    "};",
    "const C = () => (",
    "  <div />",
    ");",
    "class D {",
    "  private async run(input: string): Promise<void> {",
    "    if (input) {",
    "      return;",
    "    }",
    "  }",
    "}",
  ].join("\n"));
  expect(tsFunctions(code)).toEqual([
    { name: "A", size: 3 },
    { name: "B", size: 3 },
    { name: "C", size: 3 },
    { name: "run", size: 5 },
  ]);
});

test("hook calls count React hooks only", () => {
  const code = tsCode(
    "useState(0); React.useEffect(() => {}); useMemo<number>(() => 1, []); user(); x.useThing(); // useRef(",
  );
  expect(hookCalls(code)).toBe(3);
});

const m = (kind: Measure["kind"], key: string, size: number): Measure => ({
  kind,
  key,
  path: key.split("::")[0],
  size,
  limit: kind === "file"
    ? LIMITS.tsFile
    : kind === "hooks"
    ? LIMITS.hooks
    : LIMITS.function,
});

test("check: new offenders and growth fail; shrinking passes without touching the baseline", () => {
  const baseline = emptyBaseline();
  baseline.file["web/src/App.tsx"] = 9000;
  baseline.function["web/src/App.tsx::App"] = 2400;
  expect(
    check([
      m("file", "web/src/App.tsx", 8000),
      m("function", "web/src/App.tsx::App", 2400),
    ], baseline).failures,
  )
    .toEqual([]);
  expect(check([m("file", "web/src/App.tsx", 9001)], baseline).failures[0])
    .toContain("grew past its baseline");
  expect(check([m("file", "web/src/New.tsx", 900)], baseline).failures[0])
    .toContain("new file over its limit");
  expect(check([m("file", "web/src/Small.tsx", 100)], baseline).failures)
    .toEqual([]);
  expect(check([m("file", "web/src/App.tsx", 8000)], baseline).lowerable).toBe(
    2,
  );
});

test("update lowers sizes, drops entities under the limit and admits pure moves within an area's freed allowance", () => {
  const baseline = emptyBaseline();
  baseline.file["web/src/App.tsx"] = 9000;
  baseline.function["web/src/App.tsx::SessionList"] = 2500;
  baseline.function["web/src/App.tsx::Gone"] = 300;
  // SessionList moved unchanged into its own file.
  const moved = update([
    m("file", "web/src/App.tsx", 6500),
    m("file", "web/src/sessions/SessionList.tsx", 2500),
    m("function", "web/src/sessions/SessionList.tsx::SessionList", 2500),
  ], baseline);
  expect(moved.refused).toEqual([]);
  expect(moved.baseline.file).toEqual({
    "web/src/App.tsx": 6500,
    "web/src/sessions/SessionList.tsx": 2500,
  });
  expect(moved.baseline.function).toEqual({
    "web/src/sessions/SessionList.tsx::SessionList": 2500,
  });
  // A genuinely new offender without freed allowance is refused.
  const grown = update([
    m("file", "web/src/App.tsx", 9000),
    m("file", "web/src/Other.tsx", 900),
  ], baseline);
  expect(grown.refused[0]).toContain("no freed allowance");
  // Formatting slack absorbs a re-wrapped signature but never a new offender.
  const big = emptyBaseline();
  big.function["src/a.rs::f"] = 1000;
  big.function["src/a.rs::g"] = 1000;
  expect(
    update([
      m("function", "src/a.rs::f", 1000),
      m("function", "src/b.rs::g", 1003),
    ], big).refused,
  ).toEqual([]);
  expect(
    update([
      m("function", "src/a.rs::f", 1000),
      m("function", "src/a.rs::g", 1000),
      m("function", "src/c.rs::h", 151),
    ], big).refused[0],
  )
    .toContain("no freed allowance");
  // Allowance never crosses areas.
  const crossed = update([
    m("file", "web/src/App.tsx", 100),
    m("file", "src/new.rs", 2000),
  ], baseline);
  expect(crossed.refused[0]).toContain("src has no freed allowance");
});
