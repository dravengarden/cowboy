import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { newSessionPreset, type SessionPreset } from "./newSessionPreset";

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
