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
  return text.replace(
    /<system-reminder\b[^>]*>[\s\S]*?<\/system-reminder>/gi,
    "",
  );
}

/** Detect the structured review follow-up used by agent-side reviewer/writer
 *  loops. Require several fingerprints so a human mentioning `review_file`
 *  stays a human bubble. */
export function isAgentReviewTaskPrompt(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  return /the reviewer found issues/i.test(trimmed) &&
    /\breview_file\b/.test(trimmed) &&
    ((/status:\s*open/i.test(trimmed) && /addressed/i.test(trimmed)) ||
      /add a response field/i.test(trimmed) ||
      (/\bwontfix\b/i.test(trimmed) && /needs-user-input/i.test(trimmed)));
}

/** True when a user-role echo is only a runtime injection, not human text. */
export function isInternalRuntimePrompt(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  if (isAgentReviewTaskPrompt(trimmed)) return true;
  if (/^<system-reminder\b/i.test(trimmed)) return true;
  return stripSystemReminderBlocks(trimmed).trim() === "" &&
    SYSTEM_REMINDER_OPEN.test(trimmed);
}

export function isHumanPrompt(origin: PromptOrigin | undefined): boolean {
  return (origin?.actor ?? "human") === "human";
}

export function parsePromptOrigin(value: unknown): PromptOrigin | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as {
    actor?: unknown;
    source?: unknown;
    provider?: unknown;
  };
  if (
    record.actor !== "human" && record.actor !== "cowboy" &&
    record.actor !== "agent"
  ) {
    return undefined;
  }
  if (typeof record.source !== "string" || record.source.trim() === "") {
    return undefined;
  }
  const origin: PromptOrigin = { actor: record.actor, source: record.source };
  if (typeof record.provider === "string" && record.provider.trim() !== "") {
    origin.provider = record.provider;
  }
  return origin;
}

/** Classify a stored user-role update. Explicit `promptOrigin` wins; otherwise
 *  recover from the older `autoResumed` flag and runtime-injection markup. */
export function resolvePromptOrigin(
  update: {
    autoResumed?: unknown;
    promptOrigin?: unknown;
    [key: string]: unknown;
  },
  text: string,
): PromptOrigin {
  const explicit = parsePromptOrigin(update.promptOrigin);
  if (explicit) return explicit;
  if (isAgentReviewTaskPrompt(text)) {
    return { actor: "agent", source: "review" };
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
  return a?.actor === b?.actor && a?.source === b?.source &&
    a?.provider === b?.provider;
}

function agentReviewTaskPresentation(
  text: string,
): { title: string; raw: string } {
  const raw = text.trim();
  const title = /the reviewer found issues/i.test(raw) ||
      /\breview_file\b/.test(raw)
    ? "Addressing review findings"
    : (raw.split("\n")[0]?.trim() || "Agent task");
  return { title, raw };
}

export function runtimePromptPresentation(
  text: string,
  origin: PromptOrigin,
): { title: string; raw?: string } {
  if (origin.actor !== "agent") return { title: text };
  if (isAgentReviewTaskPrompt(text) || origin.source === "review") {
    return agentReviewTaskPresentation(text);
  }
  if (!isInternalRuntimePrompt(text)) return { title: text };
  const captured =
    /<system-reminder\b[^>]*>([\s\S]*?)(?:<\/system-reminder>|$)/i
      .exec(text);
  const inner = captured?.[1]?.trim() || text.trim();
  const firstLine = inner.split("\n")[0]?.trim() ?? inner;
  const title =
    /background task/i.test(firstLine) && /completed/i.test(firstLine)
      ? "Background task completed"
      : firstLine;
  return { title, raw: inner };
}
