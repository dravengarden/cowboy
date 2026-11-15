import type { RenderItem } from "./derive";

function durationLabel(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 90) return `${minutes} min`;
  if (minutes < 2_880) return `${(minutes / 60).toFixed(1)} h`;
  return `${Math.floor(minutes / 1_440)} days`;
}

function tokensLabel(tokens: number): string {
  return tokens >= 1_000 ? `${Math.round(tokens / 1_000)}k` : String(tokens);
}

/**
 * The row for an update where the agent's native context starts over: a fork
 * origin, a Clear (src/core.rs mark_context_cleared) or a context renewal after
 * the Provider prompt cache expired (src/context_rollover.rs). A renewal keeps
 * every earlier row; only the agent's working context restarts.
 */
export function contextBoundaryItem(u: Record<string, unknown>, key: string): RenderItem | null {
  switch (u["sessionUpdate"]) {
    case "cowboy_fork_origin": {
      const source = u["sourceSessionId"];
      if (typeof source !== "string" || !source) return null;
      const title = u["sourceTitle"];
      return {
        kind: "forked",
        key,
        sourceSessionId: source,
        sourceTitle: typeof title === "string" ? title : "",
        atMessage: typeof u["messageId"] === "string",
      };
    }
    case "context_cleared": {
      const at = u["at"];
      return { kind: "cleared", at: typeof at === "number" ? at : 0, key };
    }
    case "context_rolled_over": {
      const idle = typeof u["idleMs"] === "number" ? u["idleMs"] : 0;
      const used = typeof u["contextUsed"] === "number" ? u["contextUsed"] : 0;
      return {
        kind: "notice",
        key,
        severity: "info",
        title: "Fresh context",
        description:
          `The prompt cache expired after ${durationLabel(idle)} idle, so ` +
          `the agent continues in a fresh context instead of re-sending ` +
          `${tokensLabel(used)} tokens. It reads the history above on demand.`,
      };
    }
    default:
      return null;
  }
}
