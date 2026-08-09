import { assertEquals } from "jsr:@std/assert";
import type { ConfigOption } from "./protocol";
import {
  activeRunConfigPreset,
  runConfigPresetChanges,
  runConfigPresets,
} from "./runConfigPresets";

function selectOption(
  id: string,
  currentValue: string,
  values: readonly string[],
): ConfigOption {
  return {
    id,
    name: id,
    currentValue,
    options: values.map((value) => ({ value, name: value })),
  };
}

const openAiOptions: ConfigOption[] = [
  selectOption("model", "gpt-5.6-luna", ["gpt-5.6-sol", "gpt-5.6-luna"]),
  selectOption("reasoning_effort", "max", ["medium", "max"]),
];

const desktopSource = await Deno.readTextFile(
  new URL("./desktop/DesktopTopBarControls.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("OpenAI Codex exposes Luna Max as the default preset and Sol Medium", () => {
  const presets = runConfigPresets("codex", openAiOptions);
  assertEquals(presets.map((preset) => [preset.id, preset.isDefault]), [
    ["luna-max", true],
    ["sol-medium", false],
  ]);
  assertEquals(activeRunConfigPreset(presets, openAiOptions)?.id, "luna-max");
});

Deno.test("Claude and Codex DeepSeek expose provider-native Flash Max presets", () => {
  const claude = [
    selectOption("model", "deepseek-v4-flash[1m]", [
      "deepseek-v4-flash[1m]",
      "deepseek-v4-pro[1m]",
    ]),
    selectOption("effort", "max", ["high", "max"]),
  ];
  const codex = [
    selectOption("model", "deepseek-v4-flash", [
      "deepseek-v4-flash",
      "deepseek-v4-pro",
    ]),
    selectOption("reasoning_effort", "max", ["high", "max"]),
  ];
  assertEquals(runConfigPresets("claude-deepseek", claude)[0]?.values, {
    model: "deepseek-v4-flash[1m]",
    effort: "max",
  });
  assertEquals(runConfigPresets("codex-deepseek", codex)[0]?.values, {
    model: "deepseek-v4-flash",
    reasoning_effort: "max",
  });
});

Deno.test("presets require every live provider value", () => {
  assertEquals(runConfigPresets("codex", openAiOptions.slice(0, 1)), []);
  assertEquals(runConfigPresets("claude-code", openAiOptions), []);
});

Deno.test("preset changes omit values the session already owns", () => {
  const presets = runConfigPresets("codex", openAiOptions);
  assertEquals(runConfigPresetChanges(presets[0], openAiOptions), []);
  assertEquals(runConfigPresetChanges(presets[1], openAiOptions), [
    { configId: "model", value: "gpt-5.6-sol" },
    { configId: "reasoning_effort", value: "medium" },
  ]);
});

Deno.test("desktop and mobile expose presets with surface-native interactions", () => {
  assertEquals(desktopSource.includes("data-config-preset={index}"), true);
  assertEquals(desktopSource.includes("recommendedPresets.length === 1"), true);
  assertEquals(composerSource.includes("minHeight: 58"), true);
  assertEquals(composerSource.includes("in={showAgentDetails}"), true);
  assertEquals(composerSource.includes("setCustomizeAgent(false)"), true);
});
