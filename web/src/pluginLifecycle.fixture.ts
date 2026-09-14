/** Exact Rust projection consumed by pure and real React tests. */
import fixture from "../../tests/fixtures/plugin-lifecycle-history.json" with {
  type: "json",
};
export function lifecycleFixture(machine = "hawk", plugin = "victoria") {
  return {
    ...structuredClone(fixture),
    machine_id: machine,
    plugin_id: plugin,
  };
}
