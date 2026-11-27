import { describe, expect, test } from "bun:test";
import {
  buildRolloverChanges,
  type ContextRolloverPreferencesPayload,
  type ContextRolloverProviderChoice,
  DEFAULT_CONTEXT_ROLLOVER_VALUES,
  effectiveProviderIdleMinutes,
  formatIdleMinutesLabel,
  hasCustomRolloverPreferences,
  type PendingRolloverWrite,
  providerIdleChoices,
  providerIdleSelectionValue,
  providerRolloverDescription,
  rebuildConflictedRolloverWrite,
  reconcileLoadedRolloverPayload,
  withContextRolloverDefaults,
  withProviderIdleSelection,
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

const PROVIDERS = [CLAUDE, CODEX, CLAUDE_DEEPSEEK, ANTIGRAVITY];

describe("contextRolloverPreferences", () => {
  test("effectiveProviderIdleMinutes honors defaults, explicit null opt-outs, and host policy", () => {
    expect(effectiveProviderIdleMinutes(DEFAULT_CONTEXT_ROLLOVER_VALUES, CLAUDE)).toBe(60);
    expect(effectiveProviderIdleMinutes(DEFAULT_CONTEXT_ROLLOVER_VALUES, CODEX)).toBeNull();
    expect(effectiveProviderIdleMinutes(DEFAULT_CONTEXT_ROLLOVER_VALUES, ANTIGRAVITY)).toBeNull();
    expect(
      effectiveProviderIdleMinutes(
        { enabled: true, min_context_tokens: 80_000, idle_minutes: { codex: 240 } },
        CODEX,
      ),
    ).toBe(240);
    expect(
      effectiveProviderIdleMinutes(
        { enabled: true, min_context_tokens: 80_000, idle_minutes: { "claude-code": null } },
        CLAUDE,
      ),
    ).toBeNull();
    expect(
      effectiveProviderIdleMinutes(
        { enabled: true, min_context_tokens: 80_000, idle_minutes: { antigravity: 60 } },
        ANTIGRAVITY,
      ),
    ).toBeNull();
  });

  test("Off stores explicit null while Use default deletes the key (M4)", () => {
    expect(
      withProviderIdleSelection(DEFAULT_CONTEXT_ROLLOVER_VALUES, PROVIDERS, CODEX, "off"),
    ).toEqual({ codex: null });
    expect(
      providerIdleSelectionValue(
        { enabled: true, min_context_tokens: 80_000, idle_minutes: { codex: null } },
        CODEX,
      ),
    ).toBe("off");
    expect(providerIdleSelectionValue(DEFAULT_CONTEXT_ROLLOVER_VALUES, CODEX)).toBe("default");

    expect(
      withProviderIdleSelection(
        { enabled: true, min_context_tokens: 80_000, idle_minutes: { codex: null } },
        PROVIDERS,
        CODEX,
        "default",
      ),
    ).toEqual({});

    expect(
      withProviderIdleSelection(DEFAULT_CONTEXT_ROLLOVER_VALUES, PROVIDERS, CLAUDE, "off"),
    ).toEqual({ "claude-code": null });
  });

  test("legacy Antigravity threshold is normalized when editing another provider (M5)", () => {
    const legacy = {
      enabled: true,
      min_context_tokens: 80_000,
      idle_minutes: { antigravity: 60, "claude-code": null },
    };
    const next = withProviderIdleSelection(legacy, PROVIDERS, CODEX, 240);
    expect(next).toEqual({ "claude-code": null, codex: 240 });
    expect(Object.prototype.hasOwnProperty.call(next, "antigravity")).toBe(false);
  });

  test("409 conflict rebuild preserves concurrent edits from another device (H4)", () => {
    const initialWrite: PendingRolloverWrite = {
      datasetId: "ds-1",
      operation: "op-1",
      scope: "context_rollover",
      expected_revision: 1,
      intent: { kind: "provider_idle", providerId: "codex", selection: 240 },
      changes: buildRolloverChanges(DEFAULT_CONTEXT_ROLLOVER_VALUES, PROVIDERS, {
        kind: "provider_idle",
        providerId: "codex",
        selection: 240,
      }),
    };
    const conflictCurrent = {
      scope: "context_rollover" as const,
      revision: 2,
      values: {
        enabled: true,
        min_context_tokens: 80_000,
        idle_minutes: { "claude-code": null },
      },
    };
    const retryWrite = rebuildConflictedRolloverWrite(
      initialWrite,
      conflictCurrent,
      PROVIDERS,
      "op-2",
    );
    expect(retryWrite.datasetId).toBe("ds-1");
    expect(retryWrite.expected_revision).toBe(2);
    expect(retryWrite.operation).toBe("op-2");
    expect(retryWrite.changes).toEqual({
      idle_minutes: { "claude-code": null, codex: 240 },
    });
  });

  test("reconcileLoadedRolloverPayload never compares revisions across datasets (H5)", () => {
    const oldPayload: ContextRolloverPreferencesPayload = {
      preferences: { scope: "context_rollover", revision: 9, values: DEFAULT_CONTEXT_ROLLOVER_VALUES },
      providers: PROVIDERS,
      dataset: { dataset_id: "service-a", user_id: "u1" },
    };
    const replacementService: ContextRolloverPreferencesPayload = {
      preferences: { scope: "context_rollover", revision: 1, values: DEFAULT_CONTEXT_ROLLOVER_VALUES },
      providers: PROVIDERS,
      dataset: { dataset_id: "service-b", user_id: "u1" },
    };
    const switched = reconcileLoadedRolloverPayload(oldPayload, replacementService);
    expect(switched.datasetChanged).toBe(true);
    expect(switched.payload.dataset.dataset_id).toBe("service-b");
    expect(switched.payload.preferences.revision).toBe(1);

    const staleSameDataset: ContextRolloverPreferencesPayload = {
      ...oldPayload,
      preferences: { ...oldPayload.preferences, revision: 5 },
    };
    const same = reconcileLoadedRolloverPayload(oldPayload, staleSameDataset);
    expect(same.datasetChanged).toBe(false);
    expect(same.payload.preferences.revision).toBe(9);
  });

  test("providerIdleChoices labels default, explicit off, and canary/recommended presets", () => {
    const codexChoices = providerIdleChoices(CODEX, null);
    expect(codexChoices[0]).toEqual({ value: "default", label: "Use default (off)" });
    expect(codexChoices[1]).toEqual({ value: "off", label: "Off (never renew)" });
    expect(codexChoices.find((c) => c.value === "240")?.label).toBe("4 h · canary");

    const dsChoices = providerIdleChoices(CLAUDE_DEEPSEEK, null);
    expect(dsChoices.find((c) => c.value === "360")?.label).toBe("6 h · recommended");

    const claudeChoices = providerIdleChoices(CLAUDE, 60);
    expect(claudeChoices[0]).toEqual({ value: "default", label: "Use default (1 h)" });
    expect(claudeChoices.find((c) => c.value === "60")?.label).toBe("1 h · built-in");
  });

  test("providerRolloverDescription reflects independence and host policy", () => {
    expect(providerRolloverDescription(ANTIGRAVITY)).toContain("host policy");
    expect(providerRolloverDescription(CLAUDE_DEEPSEEK)).toContain(
      "Independent of paid DeepSeek keepalive",
    );
    expect(providerRolloverDescription(CODEX)).toContain("4 h canary");
    expect(formatIdleMinutesLabel(45)).toBe("45 min");
    expect(formatIdleMinutesLabel(240)).toBe("4 h");
    expect(hasCustomRolloverPreferences(DEFAULT_CONTEXT_ROLLOVER_VALUES)).toBe(false);
    expect(
      withContextRolloverDefaults({
        scope: "context_rollover",
        revision: 0,
        values: {} as never,
      }).values,
    ).toEqual(DEFAULT_CONTEXT_ROLLOVER_VALUES);
  });
});
