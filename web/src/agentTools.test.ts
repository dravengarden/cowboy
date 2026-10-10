import { test } from "bun:test";
import { assert } from "@std/assert";
import {
  agentLabel,
  type AgentTools,
  normalizedDefault,
  overridden,
  overrideFor,
  withTarget,
} from "./agentTools.ts";

const defaults: AgentTools = {
  schema: 1,
  matrix: { tools: true, recall: true },
  calls: {
    enabled: false,
    targets: [{ agent: "claude-code" }, { agent: "codex" }],
    default: "auto",
    max_concurrent: 4,
    max_per_session: 64,
  },
};

test("a session stores only what differs from its agent defaults", () => {
  assert(!overridden(overrideFor(defaults, defaults)));
  const next: AgentTools = {
    ...defaults,
    matrix: { tools: true, recall: false },
    calls: {
      ...defaults.calls,
      enabled: true,
      targets: withTarget(defaults.calls.targets, "codex", true, "astra-max"),
    },
  };
  const override = overrideFor(defaults, next);
  assert(overridden(override));
  assert(
    override.matrix?.recall === false && override.matrix.tools === undefined,
  );
  assert(override.calls?.enabled === true);
  assert(override.calls?.targets?.[1]?.preset === "astra-max");
  assert(override.calls?.default === undefined);
});

test("removing an agent keeps order and drops a default naming it", () => {
  const targets = withTarget(defaults.calls.targets, "claude-code", false);
  assert(targets.length === 1 && targets[0]?.agent === "codex");
  const restored = withTarget(targets, "claude-code", true);
  assert(restored.map((target) => target.agent).join() === "codex,claude-code");
  assert(
    normalizedDefault({
      ...defaults.calls,
      targets,
      default: "claude-code",
    }) === "auto",
  );
  assert(
    normalizedDefault({ ...defaults.calls, default: "codex" }) === "codex",
  );
});

test("agents are named as users know them", () => {
  assert(agentLabel("codex") === "Codex");
  assert(agentLabel("claude-code") === "Claude");
  assert(agentLabel("claude-deepseek") === "Claude Deepseek");
});
