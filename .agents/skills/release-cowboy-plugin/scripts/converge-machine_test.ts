import { assertEquals } from "jsr:@std/assert";
import {
  compareVersions,
  latestReadyReleases,
  operationId,
  planConvergence,
} from "./converge-machine.ts";

const ready = (
  plugin_id: string,
  plugin_version: string,
  digest = "sha256:aa",
) => ({
  plugin_id,
  plugin_version,
  artifact_digest: digest,
  release_state: "ready",
});
const installed = (
  plugin_id: string,
  plugin_version: string,
  active_session_leases = 0,
) => ({ plugin_id, plugin_version, state: "active", active_session_leases });

Deno.test("versions compare by ordinal, and junk never throws", () => {
  assertEquals(compareVersions("3.1.26", "3.1.9") > 0, true);
  assertEquals(compareVersions("3.1.2", "3.1.10") < 0, true);
  assertEquals(compareVersions("1.1.2", "3.1.26") < 0, true);
  assertEquals(compareVersions("1.2", "1.2.0"), 0);
  assertEquals(compareVersions("", "0.0.0"), 0);
});

Deno.test("only a ready release is a target", () => {
  const latest = latestReadyReleases([
    ready("claude-code", "3.1.25"),
    ready("claude-code", "3.1.26"),
    { ...ready("claude-code", "3.1.27"), release_state: "withdrawn" },
  ]);
  assertEquals(latest.get("claude-code")?.plugin_version, "3.1.26");
});

Deno.test("the digest is resolved from the Catalog, never authored", () => {
  const plan = planConvergence(
    "falcon",
    [ready("claude-code", "3.1.26", "sha256:e3e3")],
    [installed("claude-code", "3.1.14")],
  );
  assertEquals(plan.steps, [{
    plugin: "claude-code",
    from: "3.1.14",
    to: "3.1.26",
    digest: "sha256:e3e3",
    operationId: "falcon-claude-code-3-1-26-converge",
  }]);
});

// The identity must be a pure function of (machine, plugin, target): a retry
// after a lost response has to reuse it so the Controller can return the saved
// result instead of installing twice.
Deno.test("the operation identity is deterministic", () => {
  assertEquals(
    operationId("hawk", "claude-code", "3.1.26"),
    operationId("hawk", "claude-code", "3.1.26"),
  );
  assertEquals(
    operationId("hawk", "claude-code", "3.1.26"),
    "hawk-claude-code-3-1-26-converge",
  );
});

Deno.test("nothing is skipped silently", () => {
  const plan = planConvergence(
    "hawk",
    [
      ready("claude-code", "3.1.26"),
      ready("codex", "3.1.21"),
      ready("grok", "3.1.19"),
    ],
    [
      installed("claude-code", "3.1.26"), // already converged: not a step, not a skip
      installed("codex", "3.1.20", 2), // live worker
      installed("zed", "1.2.2"), // absent from the Catalog
      installed("grok", "9.9.9"), // ahead of the Catalog
    ],
  );
  assertEquals(plan.steps, []);
  assertEquals(plan.skipped.map((entry) => entry.plugin), [
    "codex",
    "zed",
    "grok",
  ]);
  assertEquals(plan.skipped[0]?.reason.includes("active session lease"), true);
  assertEquals(plan.skipped[2]?.reason.includes("ahead of Catalog"), true);
});

Deno.test("a plugin filter narrows the plan without widening it", () => {
  const releases = [ready("claude-code", "3.1.26"), ready("codex", "3.1.21")];
  const inventory = [
    installed("claude-code", "3.1.14"),
    installed("codex", "3.1.20"),
  ];
  const plan = planConvergence("falcon", releases, inventory, ["codex"]);
  assertEquals(plan.steps.map((step) => step.plugin), ["codex"]);
  assertEquals(plan.skipped, []);
});
