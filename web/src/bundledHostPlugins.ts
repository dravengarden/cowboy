import claudeCodeHost from "../../plugins/claude-code/host.json" with {
  type: "json",
};
import claudeDeepseekHost from "../../plugins/claude-deepseek/host.json" with {
  type: "json",
};
import codexHost from "../../plugins/codex/host.json" with { type: "json" };
import codexDeepseekHost from "../../plugins/codex-deepseek/host.json" with {
  type: "json",
};
import geminiHost from "../../plugins/gemini/host.json" with { type: "json" };
import grokHost from "../../plugins/grok/host.json" with { type: "json" };

/** First-party host.json payloads, in the same occupancy order as the controller. */
export const bundledHostPlugins: Array<{ id: string } & Record<string, unknown>> =
  [
    { id: "codex", ...(codexHost as Record<string, unknown>) },
    { id: "codex-deepseek", ...(codexDeepseekHost as Record<string, unknown>) },
    { id: "grok", ...(grokHost as Record<string, unknown>) },
    { id: "gemini", ...(geminiHost as Record<string, unknown>) },
    { id: "claude-code", ...(claudeCodeHost as Record<string, unknown>) },
    {
      id: "claude-deepseek",
      ...(claudeDeepseekHost as Record<string, unknown>),
    },
  ];
