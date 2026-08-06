import {
  providerAgentFamily,
  providerName,
  providerPresentation,
  providerSelectionName,
} from "./providerPresentation";

function assertEquals(actual: unknown, expected: unknown): void {
  if (actual !== expected) {
    throw new Error(`Expected ${String(expected)}, got ${String(actual)}`);
  }
}

Deno.test("provider presentation separates the agent from its model provider", () => {
  const openai = providerPresentation("codex");
  assertEquals(openai.agent, "Codex");
  assertEquals(openai.modelProvider, "OpenAI");
  assertEquals(openai.isolated, false);

  const deepseek = providerPresentation("codex-deepseek");
  assertEquals(deepseek.agent, "Codex");
  assertEquals(deepseek.modelProvider, "DeepSeek");
  assertEquals(deepseek.isolated, true);
  assertEquals(providerName("codex-deepseek"), "Codex · DeepSeek");
  assertEquals(providerSelectionName("codex"), "Codex · OpenAI");
  assertEquals(providerSelectionName("codex-deepseek"), "Codex · DeepSeek");

  const claudeDeepSeek = providerPresentation("claude-deepseek");
  assertEquals(claudeDeepSeek.agent, "Claude Code");
  assertEquals(claudeDeepSeek.modelProvider, "DeepSeek");
  assertEquals(claudeDeepSeek.isolated, true);
  assertEquals(providerName("claude-deepseek"), "Claude Code · DeepSeek");
  assertEquals(providerAgentFamily("claude-deepseek"), "claude-code");
  assertEquals(providerAgentFamily("codex-deepseek"), "codex");

  const reasonix = providerPresentation("reasonix-deepseek");
  assertEquals(reasonix.agent, "Reasonix");
  assertEquals(reasonix.modelProvider, "DeepSeek");
  assertEquals(reasonix.isolated, true);
  assertEquals(providerName("reasonix-deepseek"), "Reasonix · DeepSeek");
  assertEquals(providerAgentFamily("reasonix-deepseek"), "reasonix");
});

Deno.test("future provider variants degrade to machine-provided metadata", () => {
  const unknown = providerPresentation("future-agent");
  assertEquals(unknown.agent, "future-agent");
  assertEquals(unknown.modelProvider, "Custom");
});
