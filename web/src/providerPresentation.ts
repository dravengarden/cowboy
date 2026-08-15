import { currentProviderEntry } from "./providerCatalogRegistry";

export interface ProviderPresentation {
  agent: string;
  modelProvider: string;
  detail: string;
}

export function providerPresentation(
  provider: string,
  providerVersion?: string,
  providerDigest?: string,
): ProviderPresentation {
  const packaged = currentProviderEntry(provider, providerVersion, providerDigest);
  if (packaged) {
    return {
      agent: packaged.manifest.display.name,
      modelProvider: packaged.manifest.display.vendor,
      detail: packaged.manifest.display.summary,
    };
  }
  return {
    agent: provider || "Agent",
    modelProvider: "",
    detail: "Provider catalog unavailable",
  };
}

export function providerName(
  provider: string,
  providerVersion?: string,
  providerDigest?: string,
): string {
  return providerPresentation(provider, providerVersion, providerDigest).agent;
}

export function providerSelectionName(provider: string): string {
  const presentation = providerPresentation(provider);
  if (!presentation.modelProvider) return presentation.agent;
  if (presentation.agent.toLocaleLowerCase().includes(presentation.modelProvider.toLocaleLowerCase())) {
    return presentation.agent;
  }
  return `${presentation.agent} · ${presentation.modelProvider}`;
}
