import type { ProviderUiManifest } from "@cowboy/provider-ui";

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
