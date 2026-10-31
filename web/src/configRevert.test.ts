import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { configRevertSummary } from "./configRevert.ts";
import type { ConfigOption } from "./protocol.ts";

const options: ConfigOption[] = [
  {
    id: "model",
    name: "Model",
    currentValue: "sonnet",
    options: [{ value: "opus", name: "Opus" }, { value: "sonnet", name: "Sonnet" }],
  },
  {
    id: "effort",
    name: "Effort",
    currentValue: "low",
    options: [{ value: "high", name: "High" }, { value: "low", name: "Low" }],
  },
];

test("Back names the replaced values in the agent's option order", () => {
  assertEquals(configRevertSummary({ effort: "high", model: "opus" }, options), "Opus · High");
  assertEquals(configRevertSummary({}, options), "");
  // An option the agent no longer advertises still shows its raw value.
  assertEquals(configRevertSummary({ fast: true }, options), "fast on");
});
