// Cowboy managed agent calls, told to the native agent once in its system
// prompt. Without it an agent asked to "have Codex review this" starts a
// hidden `codex exec` that the user never sees, that ignores the session's
// Tools policy and model choice, and that no one can stop from Cowboy.
// It rides every turn, so it is a pointer, not a manual: the agent reads
// `cowboy review --help` or `cowboy codex --help` only when it needs them.
export const AGENT_CALLS_PROMPT = [
  "`cowboy review` gets a read-only second opinion on this worktree's change from another agent and prints only its findings (optional focus text on stdin; `--help`).",
  "Skip it by default and decide for yourself: use it at most once per task, before reporting, only when a missed bug would be costly and your own checks cannot rule it out (security, permissions, concurrency, persistence or migrations, data deletion, money, large cross-cutting refactors).",
  "Never for small, mechanical, docs, test-only or exploratory edits.",
  "Fix real findings and mention them briefly; after a `skipped:` line, or if the command is missing, say nothing.",
  "If the user asks for a review, run `cowboy review --ask`.",
  "Other delegation: `cowboy codex --help`. Never run `codex exec`, `codex review` or `claude -p` yourself; they run hidden from the user.",
].join(" ");
