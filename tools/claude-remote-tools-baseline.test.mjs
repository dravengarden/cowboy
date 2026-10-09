// The plugin's disposition of every native tool, against the pinned CLI's
// default inventory (tools/claude_tools_native_baseline.json, written by
// tools/claude_tools_native_probe.py). A candidate CLI that adds a tool fails
// here until the tool is classified; one that changes a tool's schema or
// description shows in the baseline diff.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { NATIVE_TOOLS } from "../plugins/claude-code/runtime/tools.mjs";
import {
  FORBIDDEN_TOOLS,
  NATIVE_PASSTHROUGH,
} from "../plugins/claude-code/runtime/launch.mjs";

const inventory = Object.keys(
  JSON.parse(
    readFileSync(
      new URL("./claude_tools_native_baseline.json", import.meta.url),
      "utf8",
    ),
  ).default_tools,
);

// Native tools the plugin neither routes, runs on the runtime nor disallows:
// they are not in the session's tool set, so the model never sees them.
const NOT_OFFERED = ["DesignSync", "ListAgents", "ScheduleWakeup", "Workflow"];

test("every native tool has a disposition", () => {
  const classified = new Set([
    ...NATIVE_TOOLS,
    ...NATIVE_PASSTHROUGH,
    ...FORBIDDEN_TOOLS,
    ...NOT_OFFERED,
  ]);
  assert.deepEqual(inventory.filter((tool) => !classified.has(tool)), []);
});

test("every tool the plugin offers exists natively", () => {
  // Glob and Grep are routed when a session enables them; native's default
  // set has neither (searches use Bash), so the plugin does not advertise them.
  for (const tool of [...NATIVE_TOOLS, ...NATIVE_PASSTHROUGH]) {
    if (["Glob", "Grep"].includes(tool)) {
      assert.ok(!inventory.includes(tool), tool);
    } else {
      assert.ok(inventory.includes(tool), tool);
    }
  }
});

test("dispositions do not overlap and name no stale tool", () => {
  const lists = [
    NATIVE_TOOLS,
    NATIVE_PASSTHROUGH,
    FORBIDDEN_TOOLS,
    NOT_OFFERED,
  ];
  const all = lists.flat();
  assert.equal(new Set(all).size, all.length);
  for (const tool of [...FORBIDDEN_TOOLS, ...NOT_OFFERED]) {
    // Computer and ToolSearch appear only with their features enabled.
    if (!["Computer", "ToolSearch"].includes(tool)) {
      assert.ok(inventory.includes(tool), tool);
    }
  }
});
