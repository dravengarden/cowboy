import {
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
});

Deno.test("future provider variants degrade to machine-provided metadata", () => {
  const unknown = providerPresentation("claude-deepseek");
  assertEquals(unknown.agent, "claude-deepseek");
  assertEquals(unknown.modelProvider, "Custom");
});
