/** Who placed a user-role prompt on the timeline. New sources add a `source`
 *  string; they do not invent another actor. */
export type PromptActor = "human" | "cowboy" | "agent";

export interface PromptOrigin {
  actor: PromptActor;
  /** Stable source id. Known values: composer, auto-resume, schedule, runtime, review. */
  source: string;
  /** Provider id when `actor` is `agent`, e.g. `grok`. */
  provider?: string;
}

export const HUMAN_COMPOSER_ORIGIN: PromptOrigin = {
  actor: "human",
  source: "composer",
};

const SYSTEM_REMINDER_OPEN = /<system-reminder\b/i;

export function stripSystemReminderBlocks(text: string): string {
  return text.replace(/<system-reminder\b[^>]*>[\s\S]*?<\/system-reminder>/gi, "");
}

/** Grok Build's design-review / writer loop injects a user-role prompt
 *  without `<system-reminder>` tags. Require several fingerprints so a
 *  human mentioning "review_file" stays a human bubble. */
export function isGrokRuntimeTaskPrompt(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  let hits = 0;
  if (/\/tmp\/grok-\d+\//.test(trimmed)) hits += 2;
  if (/grok-design-(?:review|doc)-[a-z0-9]+\.md/i.test(trimmed)) hits += 2;
  if (/the reviewer found issues/i.test(trimmed)) hits += 2;
  if (/\breview_file\b/.test(trimmed)) hits += 1;
  if (/status:\s*open\s*->\s*addressed/i.test(trimmed)) hits += 1;
  if (/add a response field/i.test(trimmed)) hits += 1;
  if (/\bwontfix\b/i.test(trimmed) && /needs-user-input/i.test(trimmed)) {
    hits += 1;
  }
  return hits >= 3;
}

/** True when a user-role echo is only a runtime injection, not human text. */
export function isInternalRuntimePrompt(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  if (isGrokRuntimeTaskPrompt(trimmed)) return true;
  if (/^<system-reminder\b/i.test(trimmed)) return true;
  return stripSystemReminderBlocks(trimmed).trim() === "" &&
    SYSTEM_REMINDER_OPEN.test(trimmed);
}

export function isHumanPrompt(origin: PromptOrigin | undefined): boolean {
  return (origin?.actor ?? "human") === "human";
}

export function parsePromptOrigin(value: unknown): PromptOrigin | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as { actor?: unknown; source?: unknown; provider?: unknown };
  if (record.actor !== "human" && record.actor !== "cowboy" && record.actor !== "agent") {
    return undefined;
  }
  if (typeof record.source !== "string" || record.source.trim() === "") return undefined;
  const origin: PromptOrigin = { actor: record.actor, source: record.source };
  if (typeof record.provider === "string" && record.provider.trim() !== "") {
    origin.provider = record.provider;
  }
  return origin;
}

/** Classify a stored user-role update. Explicit `promptOrigin` wins; otherwise
 *  recover from the older `autoResumed` flag and runtime-injection markup. */
export function resolvePromptOrigin(
  update: { autoResumed?: unknown; promptOrigin?: unknown; [key: string]: unknown },
  text: string,
): PromptOrigin {
  const explicit = parsePromptOrigin(update.promptOrigin);
  if (explicit) return explicit;
  if (isGrokRuntimeTaskPrompt(text)) {
    return { actor: "agent", source: "review", provider: "grok" };
  }
  if (isInternalRuntimePrompt(text)) {
    return { actor: "agent", source: "runtime" };
  }
  if (update.autoResumed === true) {
    return { actor: "cowboy", source: "auto-resume" };
  }
  return HUMAN_COMPOSER_ORIGIN;
}

export function samePromptOrigin(
  a: PromptOrigin | undefined,
  b: PromptOrigin | undefined,
): boolean {
  return a?.actor === b?.actor && a?.source === b?.source && a?.provider === b?.provider;
}

function grokRuntimeTaskPresentation(text: string): { title: string; raw: string } {
  const raw = text.trim();
  const title = /the reviewer found issues/i.test(raw) ||
      /\breview_file\b/.test(raw)
    ? "Addressing review findings"
    : (raw.split("\n")[0]?.trim() || "Grok task");
  return { title, raw };
}

export function runtimePromptPresentation(
  text: string,
  origin: PromptOrigin,
): { title: string; raw?: string } {
  if (origin.actor !== "agent") return { title: text };
  if (isGrokRuntimeTaskPrompt(text) || origin.source === "review") {
    return grokRuntimeTaskPresentation(text);
  }
  if (!isInternalRuntimePrompt(text)) return { title: text };
  const captured = /<system-reminder\b[^>]*>([\s\S]*?)(?:<\/system-reminder>|$)/i
    .exec(text);
  const inner = captured?.[1]?.trim() || text.trim();
  const firstLine = inner.split("\n")[0]?.trim() ?? inner;
  const title = /background task/i.test(firstLine) && /completed/i.test(firstLine)
    ? "Background task completed"
    : firstLine;
  return { title, raw: inner };
}
