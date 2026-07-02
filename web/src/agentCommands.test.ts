import { strict as assert } from "node:assert";
import type { AvailableCommand } from "./protocol";
import { resolveSessionAction } from "./agentCommands";

const cmds = (...names: string[]): AvailableCommand[] =>
  names.map((name) => ({ name, description: "" }));

// Per-provider defaults fire when the agent advertises nothing (cold-start window
// before the first available_commands_update).
Deno.test("defaults: claude-code compact + clear", () => {
  assert.equal(resolveSessionAction("compact", "claude-code", [])?.command, "/compact");
  assert.equal(resolveSessionAction("clear", "claude-code", [])?.command, "/clear");
});

Deno.test("defaults: codex clears via /new, compacts via /compact", () => {
  assert.equal(resolveSessionAction("compact", "codex", [])?.command, "/compact");
  assert.equal(resolveSessionAction("clear", "codex", [])?.command, "/new");
});

Deno.test("defaults: gemini compacts via /compress", () => {
  assert.equal(resolveSessionAction("compact", "gemini", [])?.command, "/compress");
  assert.equal(resolveSessionAction("clear", "gemini", [])?.command, "/clear");
});

// The advertised list is authoritative: it overrides the provider default, so a
// gemini agent that actually advertises `compact` uses THAT, not the `/compress`
// default.
Deno.test("advertised command overrides the provider default", () => {
  assert.equal(
    resolveSessionAction("compact", "gemini", cmds("compact"))?.command,
    "/compact",
  );
});

// Alias family: a differently-spelled-but-same-concept advertised command matches.
Deno.test("alias match resolves to the agent's own spelling", () => {
  assert.equal(
    resolveSessionAction("compact", "claude-code", cmds("summarize"))?.command,
    "/summarize",
  );
  assert.equal(
    resolveSessionAction("clear", "claude-code", cmds("reset"))?.command,
    "/reset",
  );
});

// Matching is case-insensitive, but the sent command keeps the agent's casing.
Deno.test("alias match is case-insensitive, preserves advertised casing", () => {
  assert.equal(
    resolveSessionAction("compact", "codex", cmds("Compact"))?.command,
    "/Compact",
  );
});

// Unknown provider with nothing advertised → null (button hides, no bogus send).
Deno.test("unknown provider with no advertised command → null", () => {
  assert.equal(resolveSessionAction("compact", "mystery-agent", []), null);
  assert.equal(resolveSessionAction("clear", "mystery-agent", []), null);
});

// …but an unknown provider that DOES advertise a matching command still works.
Deno.test("unknown provider still resolves via advertised list", () => {
  assert.equal(
    resolveSessionAction("clear", "mystery-agent", cmds("clear"))?.command,
    "/clear",
  );
});

// Clear is flagged destructive (drives the red confirm button); compact is not.
Deno.test("destructive flag: clear yes, compact no", () => {
  assert.equal(resolveSessionAction("clear", "claude-code", [])?.destructive, true);
  assert.equal(resolveSessionAction("compact", "claude-code", [])?.destructive, false);
});
