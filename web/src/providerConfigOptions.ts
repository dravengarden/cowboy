import type { ProviderUiManifest } from "../../components/provider-ui/src/index.ts";
import type { ConfigOption, Status } from "./protocol";
import { currentProviderEntry } from "./providerCatalogRegistry";

export type ProviderConfigOptionPresentation =
  ProviderUiManifest["configuration"]["options"][number];

const STANDARD_OPTION_ORDER: Readonly<Record<string, number>> = {
  mode: 0,
  session_mode: 0,
  permission_mode: 1,
  model: 2,
  effort: 5,
  reasoning_effort: 5,
  fast: 6,
  fast_mode: 6,
};

export function currentConfigOptionName(
  option: ConfigOption | undefined,
): string | null {
  if (!option) return null;
  return option.options.find((candidate) =>
    String(candidate.value) === String(option.currentValue)
  )?.name ?? String(option.currentValue);
}

// The Controller already projects the signed Provider configuration behavior.
// The UI only clones the portable options so callers cannot mutate protocol state.
export function providerConfigOptions(
  _provider: string | undefined,
  options: readonly ConfigOption[],
): ConfigOption[] {
  return [...options];
}

/** Resolve option UI policy from the exact signed Provider package. */
export function providerConfigOptionPresentations(
  provider: string | undefined,
  providerVersion?: string | undefined,
  providerDigest?: string | undefined,
): ReadonlyMap<string, ProviderConfigOptionPresentation> {
  if (!provider) return new Map();
  const declarations = currentProviderEntry(
    provider,
    providerVersion,
    providerDigest,
  )?.manifest.configuration.options ?? [];
  return new Map(
    declarations.map((declaration) => [declaration.id, declaration]),
  );
}

/** Apply a closed lifecycle policy without teaching Cowboy any Provider ids. */
export function providerConfigOptionDisabled(
  status: Status,
  presentation: ProviderConfigOptionPresentation | undefined,
): boolean {
  if (presentation?.availability === "idle_or_stopped") {
    return status === "busy" || status === "starting";
  }
  return status === "exited" || status === "crashed" ||
    status === "interrupted";
}

export function providerConfigOptionOrder(
  option: ConfigOption,
  presentation: ProviderConfigOptionPresentation | undefined,
): number {
  return presentation?.order ??
    STANDARD_OPTION_ORDER[option.id] ?? Number.MAX_SAFE_INTEGER;
}

export function providerConfigSurfaceDisabled(
  status: Status,
  options: readonly ConfigOption[],
  presentations: ReadonlyMap<string, ProviderConfigOptionPresentation>,
): boolean {
  return options.every((option) =>
    providerConfigOptionDisabled(status, presentations.get(option.id))
  );
}
