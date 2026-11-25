import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  compatiblePresetOverride,
  inheritedSessionPreset,
  newSessionPreset,
  type SessionPreset,
} from "./newSessionPreset";

const presets: SessionPreset[] = [
  { id: "other", name: "Other", detail: "", is_default: false, values: {} },
  {
    id: "opus",
    name: "Opus",
    detail: "Recommended",
    is_default: true,
    values: {},
  },
];

test("new sessions select the declared default rather than the first preset", () => {
  assertEquals(
    newSessionPreset("claude-code", presets, undefined, undefined),
    "claude-code/opus",
  );
  assertEquals(
    newSessionPreset("codex", presets, null, undefined),
    "codex/opus",
  );
});

test("directory inheritance and explicit choices take precedence over Provider defaults", () => {
  assertEquals(
    newSessionPreset("claude-code", presets, "claude-code/other", undefined),
    "claude-code/other",
  );
  assertEquals(
    newSessionPreset(
      "claude-code",
      presets,
      "claude-code/other",
      "claude-code/opus",
    ),
    "claude-code/opus",
  );
  assertEquals(
    newSessionPreset("claude-code", presets, "claude-code/other", ""),
    "",
  );
  assertEquals(
    newSessionPreset("claude-code", presets, "codex/missing", undefined),
    "codex/missing",
  );
});

test("an absent declared default uses Agent default without inventing a model", () => {
  assertEquals(newSessionPreset("codex", [], undefined, undefined), "");
  assertEquals(
    newSessionPreset("codex", presets.slice(0, 1), undefined, undefined),
    "",
  );
});

test("switching AI installation drops another Provider's directory preset and picks the new Provider's recommended preset", () => {
  const agyPresets: SessionPreset[] = [
    {
      id: "gemini-neon-high",
      name: "Gemini 4 Neon · High",
      detail: "",
      is_default: true,
      values: {},
    },
    {
      id: "claude-sonnet-high",
      name: "Claude Sonnet 5.5 · High",
      detail: "",
      is_default: false,
      values: {},
    },
  ];
  const sugerDefaults = {
    provider: "claude-code",
    preset: "claude-code/sonnet-xhigh",
  };
  // Same provider inherits the directory preset.
  assertEquals(
    inheritedSessionPreset("claude-code", sugerDefaults, false),
    "claude-code/sonnet-xhigh",
  );
  assertEquals(
    inheritedSessionPreset("claude-code", sugerDefaults, true),
    "claude-code/sonnet-xhigh",
  );
  // Switching to Antigravity drops Claude Code's preset and selects Antigravity's default.
  const agyInherited = inheritedSessionPreset(
    "antigravity",
    sugerDefaults,
    true,
  );
  assertEquals(agyInherited, undefined);
  assertEquals(
    newSessionPreset("antigravity", agyPresets, agyInherited, undefined),
    "antigravity/gemini-neon-high",
  );
  // Even without an explicit installation click (e.g. project moved to a machine without Claude),
  // a directory preset paired with Claude Code is not applied to Antigravity.
  assertEquals(
    inheritedSessionPreset("antigravity", sugerDefaults, false),
    undefined,
  );
  // A directory with only preset (implicit claude-code) also drops cleanly for another provider.
  assertEquals(
    inheritedSessionPreset(
      "antigravity",
      { preset: "claude-code/sonnet-xhigh" },
      false,
    ),
    undefined,
  );
  // A retired preset belonging to the active provider still surfaces as incompatible.
  assertEquals(
    inheritedSessionPreset(
      "antigravity",
      { provider: "antigravity", preset: "antigravity/retired" },
      false,
    ),
    "antigravity/retired",
  );
  // A genuinely misconfigured directory (default provider paired with a foreign preset) still surfaces.
  assertEquals(
    inheritedSessionPreset(
      "claude-code",
      { provider: "claude-code", preset: "codex/missing" },
      false,
    ),
    "codex/missing",
  );
  // Loading state (empty provider) does not surface a transient incompatible preset.
  assertEquals(inheritedSessionPreset("", sugerDefaults, false), undefined);
});

test("compatiblePresetOverride keeps same-provider overrides and Agent default, and clears foreign overrides", () => {
  assertEquals(
    compatiblePresetOverride("antigravity", "claude-code/sonnet-xhigh"),
    undefined,
  );
  assertEquals(
    compatiblePresetOverride("antigravity", "antigravity/claude-sonnet-high"),
    "antigravity/claude-sonnet-high",
  );
  assertEquals(compatiblePresetOverride("antigravity", ""), "");
  assertEquals(compatiblePresetOverride("antigravity", undefined), undefined);
});
