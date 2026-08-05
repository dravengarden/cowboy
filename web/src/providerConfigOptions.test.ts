import { assertEquals } from "jsr:@std/assert";
import type { ConfigOption } from "./protocol";
import { providerConfigOptions } from "./providerConfigOptions";

Deno.test("Claude DeepSeek exposes real models and distinct effort behavior", () => {
  const options: ConfigOption[] = [{
    id: "model",
    name: "Model",
    currentValue: "haiku",
    options: [
      { value: "default", name: "Default (recommended)" },
      { value: "opus", name: "deepseek-v4-pro[1m]" },
      { value: "sonnet", name: "deepseek-v4-flash[1m]" },
      { value: "haiku", name: "deepseek-v4-flash" },
      { value: "deepseek-v4-flash[1m]", name: "deepseek-v4-flash[1m]" },
      { value: "deepseek-v4-pro[1m]", name: "deepseek-v4-pro[1m]" },
    ],
  }, {
    id: "effort",
    name: "Effort",
    currentValue: "default",
    options: ["default", "low", "medium", "high", "xhigh", "max"].map(
      (value) => ({ value, name: value }),
    ),
  }];
  const normalized = providerConfigOptions("claude-deepseek", options);
  assertEquals(normalized[0]?.options.map((option) => option.name), [
    "Default · Flash (recommended)",
    "deepseek-v4-flash[1m]",
    "deepseek-v4-pro[1m]",
  ]);
  assertEquals(normalized[0]?.currentValue, "deepseek-v4-flash[1m]");
  assertEquals(normalized[1]?.options.map((option) => option.value), [
    "default",
    "high",
    "max",
  ]);
  assertEquals(normalized[1]?.options[0]?.name, "Automatic (recommended)");
});

Deno.test("ordinary providers keep their advertised ACP options", () => {
  const options: ConfigOption[] = [{
    id: "effort",
    name: "Effort",
    currentValue: "low",
    options: [{ value: "low", name: "Low" }],
  }];
  assertEquals(providerConfigOptions("claude-code", options), options);
});
