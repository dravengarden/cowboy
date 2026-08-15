import { providerPresentation } from "./providerPresentation";
import type { PromptOrigin } from "./promptOrigin";

export function originProviderId(
  origin: PromptOrigin,
  sessionProvider: string,
): string {
  return origin.provider || sessionProvider;
}

/** Short, human-facing sender name for an agent-origin prompt. */
export function agentOriginDisplayName(
  providerId: string,
  providerVersion?: string,
  providerDigest?: string,
): string {
  return providerPresentation(providerId, providerVersion, providerDigest).agent;
}

export function cowboyOriginCaption(origin: PromptOrigin): string {
  if (origin.source === "auto-resume") return "Auto-resumed the interrupted turn";
  if (origin.source === "schedule") return "Scheduled wakeup";
  return `Cowboy · ${origin.source}`;
}

export function agentOriginSourceLabel(origin: PromptOrigin): string {
  return origin.source === "runtime" ? "runtime" : origin.source;
}
