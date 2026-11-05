import { expect, test } from "bun:test";
import {
  type Baseline,
  buildGraph,
  check,
  type CrateSource,
  cycles,
  references,
  resolve,
  snapshot,
} from "./check-module-layers.ts";
import { rustCode, stripRustTests } from "./check-structure.ts";

const code = (source: string): string => stripRustTests(rustCode(source)).code;

function crate(
  name: string,
  root: string,
  modules: Record<string, Record<string, string>>,
): CrateSource {
  return {
    name,
    root: code(root),
    modules: new Map(
      Object.entries(modules).map(([module, files]) => [
        module,
        Object.entries(files).map(([path, source]) => ({
          path,
          code: code(source),
        })),
      ]),
    ),
  };
}

test("references see crate paths, grouped trees, aliases and root-climbing super chains, not comments or tests", () => {
  const source = code([
    "use crate::{core::Hub, store, config::{self, Load}};",
    "use crate::usage as metered;",
    "use super::super::machine_control::Lane;",
    "use super::sibling::Item;",
    "/// See [`crate::supervisor::Supervisor`].",
    'const PATH: &str = "crate::logs";',
    "fn f() { cowboy_wire::machine_protocol::negotiate(); crate::scheduler::tick(); }",
    "#[cfg(test)]",
    "mod tests { use crate::fixtures::Thing; }",
  ].join("\n"));
  const refs = references(source, 2, ["cowboy_wire"]);
  expect([...refs.local].sort()).toEqual([
    "config",
    "core",
    "machine_control",
    "scheduler",
    "store",
    "usage",
  ]);
  expect([...refs.external]).toEqual(["cowboy_wire::machine_protocol"]);
  // From a file directly under the crate root a single super:: reaches the root.
  expect([...references(code("use super::store::Row;"), 1, []).local]).toEqual([
    "store",
  ]);
  expect([...references(code("use super::store::Row;"), 2, []).local]).toEqual(
    [],
  );
});

test("names resolve through explicit and glob re-exports to the owning crate's module", () => {
  const crates = new Map<string, CrateSource>([
    [
      "cowboy_wire",
      crate("cowboy_wire", "", {
        machine_protocol: { "w/machine_protocol.rs": "" },
      }),
    ],
    [
      "cowboy",
      crate("cowboy", "pub use cowboy_wire::machine_protocol;\npub mod core;", {
        core: { "s/core.rs": "" },
      }),
    ],
    [
      "cowboy_server",
      crate("cowboy_server", "pub use cowboy::*;", {
        server: { "c/server.rs": "" },
      }),
    ],
  ]);
  expect(resolve(crates, "cowboy_server", "core")).toBe("cowboy::core");
  expect(resolve(crates, "cowboy_server", "machine_protocol")).toBe(
    "cowboy_wire::machine_protocol",
  );
  expect(resolve(crates, "cowboy_server", "Jemalloc")).toBeNull();
});

function graphOf(
  modules: Record<string, string>,
): ReturnType<typeof buildGraph> {
  const files = Object.fromEntries(
    Object.entries(modules).map((
      [module, source],
    ) => [module, { [`src/${module}.rs`]: source }]),
  );
  return buildGraph(new Map([["cowboy", crate("cowboy", "", files)]]));
}

test("cycles are found and a recorded cycle may shrink but not grow, merge or tighten", () => {
  const recorded = graphOf({
    core: "use crate::store::Row;",
    store: "use crate::core::Meta;",
    config: "",
    usage: "use crate::config::X;",
  });
  expect(cycles(recorded)).toEqual([["cowboy::core", "cowboy::store"]]);
  const baseline: Baseline = snapshot(recorded);
  expect(check(recorded, baseline)).toEqual([]);

  // An ordinary new edge outside any cycle is fine.
  expect(
    check(
      graphOf({
        ...{
          core: "use crate::store::Row; use crate::config::Y;",
          store: "use crate::core::Meta;",
        },
        config: "",
        usage: "use crate::config::X;",
      }),
      baseline,
    ),
  ).toEqual([]);

  // Pulling another module into the cycle fails.
  const grown = graphOf({
    core: "use crate::store::Row;",
    store: "use crate::core::Meta; use crate::usage::U;",
    config: "",
    usage: "use crate::config::X; use crate::core::Meta;",
  });
  expect(check(grown, baseline)[0]).toContain("newly entangled: cowboy::usage");

  // A brand-new cycle elsewhere fails.
  const fresh = graphOf({
    core: "use crate::store::Row;",
    store: "use crate::core::Meta;",
    config: "use crate::usage::U;",
    usage: "use crate::config::X;",
  });
  expect(check(fresh, baseline)[0]).toContain(
    "new or enlarged dependency cycle",
  );

  // Breaking the cycle passes.
  expect(
    check(
      graphOf({
        core: "",
        store: "use crate::core::Meta;",
        config: "",
        usage: "",
      }),
      baseline,
    ),
  ).toEqual([]);
});

test("a new edge between two members of the same cycle fails", () => {
  const recorded = graphOf({
    a: "use crate::b::X;",
    b: "use crate::c::X;",
    c: "use crate::a::X;",
  });
  const baseline = snapshot(recorded);
  const tightened = graphOf({
    a: "use crate::b::X; use crate::c::Y;",
    b: "use crate::c::X;",
    c: "use crate::a::X;",
  });
  expect(check(tightened, baseline)).toEqual([
    "new edge inside a dependency cycle: cowboy::a -> cowboy::c; depend on a narrower module or a shared contract instead",
  ]);
});
