import type { ProviderUiManifest } from "@cowboy/provider-ui";
import { useState } from "react";

export type SessionPreset =
  ProviderUiManifest["configuration"]["presets"][number];

/** Show and submit the same default, while preserving explicit overrides. */
export function newSessionPreset(
  provider: string,
  presets: readonly SessionPreset[],
  inherited: string | null | undefined,
  override: string | undefined,
): string {
  if (override !== undefined) return override;
  if (inherited) return inherited;
  const preset = presets.find((entry) => entry.is_default);
  return preset ? `${provider}/${preset.id}` : "";
}

/** The preset of Replace or fork's target agent. A staged target keeps its
 * preset; another Provider starts on its recommended one, while the same
 * Provider inherits the current model and reasoning. Changing the Provider
 * (`choose(undefined)`) returns to that default. */
export function useTargetPreset(
  staged: { preset?: string | null } | null | undefined,
  provider: string,
  sourceProvider: string,
  presets: readonly SessionPreset[],
): [string, (value: string | undefined) => void] {
  const [choice, choose] = useState<string | undefined>(
    staged ? staged.preset || "" : undefined,
  );
  const preset = newSessionPreset(
    provider,
    provider === sourceProvider ? [] : presets,
    null,
    choice,
  );
  return [preset, choose];
}
