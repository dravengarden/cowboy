export interface ProviderPresentation {
  agent: string;
  modelProvider: string;
  detail: string;
  isolated: boolean;
}

const PRESENTATIONS: Readonly<Record<string, ProviderPresentation>> = {
  codex: {
    agent: "Codex",
    modelProvider: "OpenAI",
    detail: "Standard Codex account and configuration",
    isolated: false,
  },
  "codex-deepseek": {
    agent: "Codex",
    modelProvider: "DeepSeek",
    detail: "Isolated API configuration · V4 Flash",
    isolated: true,
  },
  "claude-code": {
    agent: "Claude Code",
    modelProvider: "Anthropic",
    detail: "Standard Claude Code account and configuration",
    isolated: false,
  },
  gemini: {
    agent: "Gemini",
    modelProvider: "Google",
    detail: "Standard Gemini CLI configuration",
    isolated: false,
  },
};

export function providerPresentation(provider: string): ProviderPresentation {
  return PRESENTATIONS[provider] ?? {
    agent: provider || "Agent",
    modelProvider: "Custom",
    detail: "Machine-provided runtime",
    isolated: false,
  };
}

export function providerName(provider: string): string {
  const presentation = providerPresentation(provider);
  return presentation.isolated
    ? `${presentation.agent} · ${presentation.modelProvider}`
    : presentation.agent;
}

export function providerSelectionName(provider: string): string {
  const presentation = providerPresentation(provider);
  return `${presentation.agent} · ${presentation.modelProvider}`;
}
