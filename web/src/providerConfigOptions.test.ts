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
    currentValue: "max",
    options: ["default", "low", "medium", "high", "xhigh", "max"].map(
      (value) => ({ value, name: value }),
    ),
  }];
  const normalized = providerConfigOptions("claude-deepseek", options);
  assertEquals(normalized[0]?.options.map((option) => option.name), [
    "deepseek-v4-flash[1m]",
    "deepseek-v4-pro[1m]",
  ]);
  assertEquals(normalized[0]?.currentValue, "deepseek-v4-flash[1m]");
  assertEquals(normalized[1]?.options.map((option) => option.value), [
    "high",
    "max",
  ]);
});

Deno.test("DeepSeek custom controls remove default aliases and recommendation labels", () => {
  const options: ConfigOption[] = [{
    id: "model",
    name: "Model",
    currentValue: "default",
    options: [
      { value: "default", name: "Default (recommended)" },
      { value: "deepseek-v4-flash", name: "DeepSeek-V4-Flash" },
      { value: "deepseek-v4-pro", name: "DeepSeek-V4-Pro" },
    ],
  }, {
    id: "deepseek_cache_protection",
    name: "Cache protection",
    currentValue: true,
    options: [
      { value: true, name: "Auto · recommended" },
      { value: false, name: "Off" },
    ],
  }];
  const normalized = providerConfigOptions("codex-deepseek", options);
  assertEquals(normalized[0]?.currentValue, "deepseek-v4-flash");
  assertEquals(normalized[0]?.options.map((option) => option.name), [
    "DeepSeek-V4-Flash",
    "DeepSeek-V4-Pro",
  ]);
  assertEquals(normalized[1]?.options.map((option) => option.name), [
    "Auto",
    "Off",
  ]);
});

Deno.test("DeepSeek context options show working windows and compaction points", () => {
  const context: ConfigOption = {
    id: "deepseek_context",
    name: "Context budget",
    currentValue: "680k",
    options: ["128k", "256k", "512k", "680k", "830k", "future"].map(
      (value) => ({ value, name: value.toUpperCase() }),
    ),
  };

  const claude = providerConfigOptions("claude-deepseek", [context])[0];
  assertEquals(claude?.name, "Working context");
  assertEquals(claude?.options.map((option) => option.name), [
    "128K window · compacts at 128K",
    "256K window · compacts at 256K",
    "512K window · compacts at 512K",
    "680K window · compacts at 680K",
    "830K window · compacts at 819.2K",
    "FUTURE",
  ]);

  const codex = providerConfigOptions("codex-deepseek", [context])[0];
  assertEquals(codex?.name, "Working context");
  assertEquals(codex?.options.map((option) => option.name), [
    "128K window · compacts at 121.6K",
    "256K window · compacts at 243.2K",
    "512K window · compacts at 486.4K",
    "680K window · compacts at 646K",
    "830K window · compacts at 788.5K · large",
    "FUTURE",
  ]);
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
