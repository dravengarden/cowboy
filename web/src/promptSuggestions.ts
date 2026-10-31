// The follow-up prompt an agent predicts after a turn. Adapters publish it as
// `session_info_update._meta.promptSuggestions = {suggestions: [{text}]}`; an
// empty list clears the one the client holds (claude-agent-acp
// dist/prompt-suggestions.js, codex-acp PromptSuggestionGenerator). The update
// is runtime telemetry the daemon never stores, so the suggestion is live-only:
// a reload or reconnect simply shows none until the next turn predicts one.
// Nothing here ever sends: the composer only offers the text.
import { useSyncExternalStore } from "react";
import type { AcpUpdate, Envelope } from "./protocol";

export const PROMPT_SUGGESTIONS_META_KEY = "promptSuggestions";

const MAX_SUGGESTION_CHARS = 2_000;

/** The suggestion an update publishes: its text, `null` when it clears the
 *  held one, or `undefined` when the update says nothing about suggestions. */
export function promptSuggestionOf(
  update: AcpUpdate,
): string | null | undefined {
  if (update.sessionUpdate !== "session_info_update") return undefined;
  const meta = update["_meta"];
  if (!meta || typeof meta !== "object" || Array.isArray(meta)) {
    return undefined;
  }
  const value = (meta as Record<string, unknown>)[PROMPT_SUGGESTIONS_META_KEY];
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  const suggestions = (value as { suggestions?: unknown }).suggestions;
  if (!Array.isArray(suggestions)) return undefined;
  for (const suggestion of suggestions) {
    const text = suggestion && typeof suggestion === "object"
      ? (suggestion as { text?: unknown }).text
      : undefined;
    if (typeof text !== "string") continue;
    // One line in the composer: the adapters predict a short single prompt.
    const line = text.replace(/\s+/g, " ").trim();
    if (line) return line.slice(0, MAX_SUGGESTION_CHARS);
  }
  return null;
}

const held = new Map<string, string>();
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function set(sessionId: string, text: string | null): void {
  if (text === null ? !held.has(sessionId) : held.get(sessionId) === text) {
    return;
  }
  if (text === null) held.delete(sessionId);
  else held.set(sessionId, text);
  emit();
}

/** Follow one live envelope: a suggestion update replaces or clears the held
 *  text, and a new turn (the user's prompt echo) clears it. */
export function observePromptSuggestion(env: Envelope): void {
  if (env.kind !== "update") return;
  if (env.update.sessionUpdate === "user_message_chunk") {
    set(env.session_id, null);
    return;
  }
  const suggestion = promptSuggestionOf(env.update);
  if (suggestion !== undefined) set(env.session_id, suggestion);
}

/** The user typed, accepted it, or otherwise moved past the suggestion. */
export function dismissPromptSuggestion(sessionId: string): void {
  set(sessionId, null);
}

export function promptSuggestion(sessionId: string): string | null {
  return held.get(sessionId) ?? null;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return (): void => {
    listeners.delete(listener);
  };
}

export function usePromptSuggestion(
  sessionId: string | undefined,
): string | null {
  return useSyncExternalStore(
    subscribe,
    () => (sessionId === undefined ? null : promptSuggestion(sessionId)),
    () => null,
  );
}
