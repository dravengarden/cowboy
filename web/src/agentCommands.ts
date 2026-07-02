// Session-lifecycle actions the composer exposes as one-tap buttons: "compact"
// (summarise the running context so the agent keeps going with less history) and
// "clear" (drop the history and start fresh). Each maps to a real agent slash-
// command whose SPELLING differs per CLI, so we resolve the concrete command at
// click time from two sources, in order:
//
//   1. What the running agent advertises over ACP (`available_commands_update`).
//      This is authoritative — it follows the actual agent (and any upstream
//      rename) with zero code change here. We match by an alias set, not an
//      exact name, because the same concept is spelled differently per agent.
//   2. A per-provider default, for agents that don't advertise the command over
//      ACP. Best-effort; the advertised list always wins when present.
//
// An unknown provider with nothing advertised resolves to `null` → the button is
// hidden, so a new backend provider degrades gracefully until its mapping lands.
import type { AvailableCommand } from "./protocol";

export type SessionActionId = "compact" | "clear";

export interface SessionAction {
  id: SessionActionId;
  /** The `/command` string to send (leading slash included). */
  command: string;
  /** Button + confirm-dialog title. */
  label: string;
  /** One-line consequence, shown in the confirm dialog. */
  detail: string;
  /** Confirm button verb + colour intent ("clear" is destructive). */
  destructive: boolean;
}

// The same concept, spelled differently across the agents cowboy drives. Matched
// case-insensitively against the agent's advertised command names, so an upstream
// rename inside the alias family is picked up without touching this file.
const ALIASES: Record<SessionActionId, readonly string[]> = {
  compact: ["compact", "compress", "summarize", "summarise"],
  clear: ["clear", "new", "reset"],
};

// Per-provider fallback command NAMES (no slash) when the agent advertises no
// matching command. Kept minimal on purpose — the advertised list is the real
// source of truth; these only cover the cold-start window before the first
// `available_commands_update` arrives.
const DEFAULT_NAME: Record<string, Partial<Record<SessionActionId, string>>> = {
  "claude-code": { compact: "compact", clear: "clear" },
  "codex": { compact: "compact", clear: "new" },
  "gemini": { compact: "compress", clear: "clear" },
};

const LABEL: Record<SessionActionId, string> = {
  compact: "Compact conversation",
  clear: "Clear conversation",
};

const DETAIL: Record<SessionActionId, string> = {
  compact:
    "Summarise the conversation so far into a shorter context. The agent keeps working, with the condensed history in place of the full transcript.",
  clear:
    "Start a fresh context, discarding the conversation history so far. The transcript stays on screen, but the agent forgets it. This can't be undone.",
};

function resolveName(
  id: SessionActionId,
  provider: string,
  available: readonly AvailableCommand[],
): string | null {
  const aliases = ALIASES[id];
  const advertised = available.find((c) => aliases.includes(c.name.toLowerCase()));
  if (advertised) return advertised.name;
  return DEFAULT_NAME[provider]?.[id] ?? null;
}

/** Resolve a composer session-action to the concrete slash-command for this
 *  session's agent, or `null` when the agent offers no equivalent. */
export function resolveSessionAction(
  id: SessionActionId,
  provider: string,
  available: readonly AvailableCommand[],
): SessionAction | null {
  const name = resolveName(id, provider, available);
  if (name === null) return null;
  return {
    id,
    command: `/${name}`,
    label: LABEL[id],
    detail: DETAIL[id],
    destructive: id === "clear",
  };
}
