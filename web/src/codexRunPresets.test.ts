import { assertEquals } from "jsr:@std/assert";
import type { ConfigOption } from "./protocol";
import {
  activeCodexRunPreset,
  codexRunPresetChanges,
  codexRunPresets,
} from "./codexRunPresets";

const options: ConfigOption[] = [
  {
    id: "model",
    name: "Model",
    currentValue: "gpt-5.6-luna",
    options: [
      { value: "gpt-5.6-sol", name: "GPT-5.6-Sol" },
      { value: "gpt-5.6-luna", name: "GPT-5.6-Luna" },
    ],
  },
  {
    id: "reasoning_effort",
    name: "Reasoning effort",
    currentValue: "max",
    options: [
      { value: "medium", name: "Medium" },
      { value: "max", name: "Max" },
    ],
  },
];

const desktopSource = await Deno.readTextFile(
  new URL("./desktop/DesktopTopBarControls.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("OpenAI Codex exposes Luna Max as the default preset and Sol Medium", () => {
  const presets = codexRunPresets("codex", options);
  assertEquals(presets.map((preset) => [preset.id, preset.isDefault]), [
    ["luna-max", true],
    ["sol-medium", false],
  ]);
  assertEquals(activeCodexRunPreset(presets, options)?.id, "luna-max");
});

Deno.test("presets stay provider-native and require both live values", () => {
  assertEquals(codexRunPresets("codex-deepseek", options), []);
  assertEquals(codexRunPresets("codex", options.slice(0, 1)), []);
});

Deno.test("preset changes omit values the session already owns", () => {
  const presets = codexRunPresets("codex", options);
  assertEquals(codexRunPresetChanges(presets[0], options), []);
  assertEquals(codexRunPresetChanges(presets[1], options), [
    { configId: "model", value: "gpt-5.6-sol" },
    { configId: "reasoning_effort", value: "medium" },
  ]);
});

Deno.test("desktop and mobile expose presets with surface-native interactions", () => {
  assertEquals(desktopSource.includes("data-config-preset={index}"), true);
  assertEquals(desktopSource.includes('shortcut: "1/2"'), true);
  assertEquals(composerSource.includes("minHeight: 58"), true);
  assertEquals(
    composerSource.includes("<Collapse in={showAgentDetails}"),
    true,
  );
  assertEquals(composerSource.includes("setCustomizeAgent(false)"), true);
});
