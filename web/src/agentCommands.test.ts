import { strict as assert } from "node:assert";
import type { AvailableCommand } from "./protocol";
import { resolveSessionAction } from "./agentCommands";

const cmds = (...names: string[]): AvailableCommand[] =>
  names.map((name) => ({ name, description: "" }));

// Compact is a slash-command: per-provider default when nothing is advertised.
Deno.test("compact defaults: claude/codex use /compact, gemini /compress", () => {
  assert.equal(resolveSessionAction("compact", "claude-code", [])?.command, "/compact");
  assert.equal(
    resolveSessionAction("compact", "claude-deepseek", [])?.command,
    "/compact",
  );
  assert.equal(resolveSessionAction("compact", "codex", [])?.command, "/compact");
  assert.equal(resolveSessionAction("compact", "gemini", [])?.command, "/compress");
});

Deno.test("Reasonix hides compact until its ACP server exposes the native action", () => {
  assert.equal(resolveSessionAction("compact", "reasonix-deepseek", []), null);
});

// The advertised list overrides the default, matched by alias, agent's own casing.
Deno.test("compact prefers the advertised command (alias, case-insensitive)", () => {
  assert.equal(
    resolveSessionAction("compact", "gemini", cmds("compact"))?.command,
    "/compact",
  );
  assert.equal(
    resolveSessionAction("compact", "claude-code", cmds("Summarize"))?.command,
    "/Summarize",
  );
});

// Unknown provider with nothing advertised → no compact button.
Deno.test("compact hides for an unknown provider with no advertised command", () => {
  assert.equal(resolveSessionAction("compact", "mystery", []), null);
});

// Clear is a client-side RESET, not a slash command: always available, no
// `command`, kind "reset", destructive — regardless of provider or advertised set.
Deno.test("clear is always a reset action, no slash command", () => {
  for (const provider of ["claude-code", "codex", "gemini", "mystery"]) {
    const a = resolveSessionAction("clear", provider, []);
    assert.ok(a, `clear available for ${provider}`);
    assert.equal(a?.kind, "reset");
    assert.equal(a?.command, undefined, "clear carries no slash command");
    assert.equal(a?.destructive, true);
  }
});

Deno.test("compact is a slash action, not destructive", () => {
  const a = resolveSessionAction("compact", "claude-code", []);
  assert.equal(a?.kind, "slash");
  assert.equal(a?.destructive, false);
});
