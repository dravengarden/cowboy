import { assertEquals } from "jsr:@std/assert";
import type { ProviderUiManifest } from "../../packages/provider-ui-sdk/src/index.ts";
import type { ConfigOption } from "./protocol";
import {
  activeRunConfigPreset,
  runConfigPresetChanges,
  supportedRunConfigPresets,
} from "./runConfigPresets";

type DeclaredPreset = ProviderUiManifest["configuration"]["presets"][number];

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

const options: ConfigOption[] = [
  selectOption("model", "model-a", ["model-a", "model-b"]),
  selectOption("effort", "max", ["medium", "max"]),
];

const declared: DeclaredPreset[] = [{
  id: "recommended",
  name: "Recommended",
  detail: "Provider-owned recommendation",
  is_default: true,
  values: { model: "model-a", effort: "max" },
}, {
  id: "balanced",
  name: "Balanced",
  detail: "Provider-owned balanced mode",
  is_default: false,
  values: { model: "model-b", effort: "medium" },
}];

Deno.test("signed Provider presets project without Provider identity branches", () => {
  const presets = supportedRunConfigPresets(declared, options);
  assertEquals(presets.map((preset) => [preset.id, preset.isDefault]), [
    ["recommended", true],
    ["balanced", false],
  ]);
  assertEquals(activeRunConfigPreset(presets, options)?.id, "recommended");
});

Deno.test("presets fail closed when the live Provider surface lacks a value", () => {
  assertEquals(supportedRunConfigPresets(declared, options.slice(0, 1)), []);
  assertEquals(supportedRunConfigPresets([{ ...declared[0], values: { model: "unknown" } }], options), []);
});

Deno.test("preset changes omit values the session already owns", () => {
  const presets = supportedRunConfigPresets(declared, options);
  assertEquals(runConfigPresetChanges(presets[0], options), []);
  assertEquals(runConfigPresetChanges(presets[1], options), [
    { configId: "model", value: "model-b" },
    { configId: "effort", value: "medium" },
  ]);
});

const desktopSource = await Deno.readTextFile(
  new URL("./desktop/DesktopTopBarControls.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("desktop and mobile expose presets with surface-native interactions", () => {
  assertEquals(desktopSource.includes("data-config-preset={index}"), true);
  assertEquals(desktopSource.includes("presetShortcutLabel"), true);
  assertEquals(composerSource.includes("minHeight: 58"), true);
  assertEquals(composerSource.includes("in={showAgentDetails}"), true);
  assertEquals(composerSource.includes("setCustomizeAgent(false)"), true);
});
