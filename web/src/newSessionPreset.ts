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

/** Directory preset applicable to the currently selected Provider.
 *
 * A preset belonging to `provider` is always inherited (even when retired in
 * the installed version, so the form surfaces the incompatibility). A preset
 * belonging to another Provider is ignored when the user chose a different AI
 * installation or when the directory paired that preset with its own default
 * Provider, so switching agents falls back to the new Provider's recommended
 * preset instead of blocking creation. A genuinely misconfigured directory
 * (explicit default Provider paired with a foreign preset) still surfaces. */
export function inheritedSessionPreset(
  provider: string,
  defaults: { provider?: string | null; preset?: string | null },
  installationOverridden: boolean,
): string | undefined {
  const preset = defaults.preset;
  if (!provider || !preset) return undefined;
  const slash = preset.indexOf("/");
  const owner = slash > 0 ? preset.slice(0, slash) : "";
  if (owner === provider) return preset;
  if (
    installationOverridden ||
    (owner !== "" && defaults.provider === owner) ||
    (!defaults.provider && owner === "claude-code")
  ) {
    return undefined;
  }
  return preset;
}

/** Explicit preset override applicable to `provider`. Foreign overrides from a
 * previous Provider are ignored so switching agents never strands an
 * incompatible value; explicit `""` ("Agent default") stays intact. */
export function compatiblePresetOverride(
  provider: string,
  override: string | undefined,
): string | undefined {
  if (!override || !provider) return override;
  return override.startsWith(`${provider}/`) ? override : undefined;
}

/** Combined preset state for Create Session. */
export function resolveCreateSessionPreset(
  provider: string,
  presets: readonly SessionPreset[],
  defaults: { provider?: string | null; preset?: string | null },
  installationOverridden: boolean,
  override: string | undefined,
): {
  inherited: string | undefined;
  override: string | undefined;
  selected: string;
  available: boolean;
} {
  const inherited = inheritedSessionPreset(
    provider,
    defaults,
    installationOverridden,
  );
  const effectiveOverride = compatiblePresetOverride(provider, override);
  const selected = newSessionPreset(
    provider,
    presets,
    inherited,
    effectiveOverride,
  );
  const available = !selected ||
    presets.some((preset) => `${provider}/${preset.id}` === selected);
  return { inherited, override: effectiveOverride, selected, available };
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
