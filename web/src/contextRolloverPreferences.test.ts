import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert";
import {
  type ContextRolloverProviderChoice,
  DEFAULT_CONTEXT_ROLLOVER_VALUES,
  effectiveProviderIdleMinutes,
  formatIdleMinutesLabel,
  providerIdleChoices,
  providerRolloverDescription,
  withContextRolloverDefaults,
  withProviderIdleMinutes,
} from "./contextRolloverPreferences";

const CLAUDE: ContextRolloverProviderChoice = {
  id: "claude-code",
  name: "Claude Code",
  builtin_idle_minutes: 60,
  recommended_idle_minutes: 60,
  policy_restricted: false,
  restriction: null,
  keepalive_independent: false,
};

const CODEX: ContextRolloverProviderChoice = {
  id: "codex",
  name: "Codex",
  builtin_idle_minutes: null,
  recommended_idle_minutes: 240,
  policy_restricted: false,
  restriction: null,
  keepalive_independent: false,
};

const CLAUDE_DEEPSEEK: ContextRolloverProviderChoice = {
  id: "claude-deepseek",
  name: "Claude Code · DeepSeek",
  builtin_idle_minutes: null,
  recommended_idle_minutes: 360,
  policy_restricted: false,
  restriction: null,
  keepalive_independent: true,
};

const ANTIGRAVITY: ContextRolloverProviderChoice = {
  id: "antigravity",
  name: "Antigravity",
  builtin_idle_minutes: null,
  recommended_idle_minutes: null,
  policy_restricted: true,
  restriction: "Automatic context renewal is disabled for Antigravity accounts by host policy.",
  keepalive_independent: false,
};

test("effectiveProviderIdleMinutes resolves defaults, explicit overrides, and policy restrictions", () => {
  assertEquals(effectiveProviderIdleMinutes(DEFAULT_CONTEXT_ROLLOVER_VALUES, CLAUDE), 60);
  assertEquals(effectiveProviderIdleMinutes(DEFAULT_CONTEXT_ROLLOVER_VALUES, CODEX), null);
  assertEquals(effectiveProviderIdleMinutes(DEFAULT_CONTEXT_ROLLOVER_VALUES, ANTIGRAVITY), null);

  const custom = {
    enabled: true,
    min_context_tokens: 80_000,
    idle_minutes: { "claude-code": null, codex: 240, antigravity: 60 },
  };
  assertEquals(effectiveProviderIdleMinutes(custom, CLAUDE), null);
  assertEquals(effectiveProviderIdleMinutes(custom, CODEX), 240);
  assertEquals(effectiveProviderIdleMinutes(custom, ANTIGRAVITY), null);
});

test("withProviderIdleMinutes elides built-in defaults and refuses policy-restricted writes", () => {
  // Opting into Codex 240m canary stores explicit 240.
  const optedIn = withProviderIdleMinutes(DEFAULT_CONTEXT_ROLLOVER_VALUES, CODEX, 240);
  assertEquals(optedIn, { codex: 240 });

  // Returning Codex to Off (its built-in default) removes the redundant key.
  const resetCodex = withProviderIdleMinutes(
    { ...DEFAULT_CONTEXT_ROLLOVER_VALUES, idle_minutes: optedIn },
    CODEX,
    null,
  );
  assertEquals(resetCodex, {});

  // Disabling Claude Code stores explicit null because its built-in default is 60.
  const disabledClaude = withProviderIdleMinutes(DEFAULT_CONTEXT_ROLLOVER_VALUES, CLAUDE, null);
  assertEquals(disabledClaude, { "claude-code": null });

  // Restoring Claude Code to 60 elides the key.
  assertEquals(
    withProviderIdleMinutes(
      { ...DEFAULT_CONTEXT_ROLLOVER_VALUES, idle_minutes: disabledClaude },
      CLAUDE,
      60,
    ),
    {},
  );

  // Policy-restricted Antigravity never stores a positive idle threshold.
  assertEquals(
    withProviderIdleMinutes(
      { ...DEFAULT_CONTEXT_ROLLOVER_VALUES, idle_minutes: { antigravity: 60 } },
      ANTIGRAVITY,
      60,
    ),
    {},
  );
});

test("providerIdleChoices labels canary, recommended, default, and custom thresholds", () => {
  const codexChoices = providerIdleChoices(CODEX, null);
  assertEquals(codexChoices[0], { value: "off", label: "Off · default" });
  assertEquals(
    codexChoices.find((choice) => choice.value === "240"),
    { value: "240", label: "4 h · canary" },
  );

  const deepseekChoices = providerIdleChoices(CLAUDE_DEEPSEEK, 180);
  assertEquals(
    deepseekChoices.find((choice) => choice.value === "180"),
    { value: "180", label: "3 h" },
  );
  assertEquals(
    deepseekChoices.find((choice) => choice.value === "360"),
    { value: "360", label: "6 h · recommended" },
  );

  assertEquals(formatIdleMinutesLabel(45), "45 min");
  assertEquals(formatIdleMinutesLabel(240), "4 h");
  assertEquals(
    providerRolloverDescription(CLAUDE_DEEPSEEK).includes("Independent from DeepSeek cache protection"),
    true,
  );
  assertEquals(
    providerRolloverDescription(ANTIGRAVITY).includes("host policy"),
    true,
  );

  const normalized = withContextRolloverDefaults({
    scope: "context_rollover",
    revision: 0,
    values: {} as never,
  });
  assertEquals(normalized.values, DEFAULT_CONTEXT_ROLLOVER_VALUES);
});
