// Cowboy managed agent calls, told to the native agent once in its system
// prompt. Without it an agent asked to "have Codex review this" starts a
// hidden `codex exec` that the user never sees, that ignores the session's
// Tools policy and model choice, and that no one can stop from Cowboy.
export const AGENT_CALLS_PROMPT = [
  "To have another agent review or analyze work (for example when asked to call Codex), use Cowboy instead of running `codex exec`, `codex review` or `claude -p` yourself.",
  "Run `cowboy codex --request-file - [--preset ID]` with this JSON on stdin:",
  '{"schema":1,"request_id":"<new unique id>","purpose":"review|design_review|analysis","instruction":"<self-contained task>","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}.',
  "It prints a JSON envelope with a call_id; then run `cowboy call wait <call_id>` until it is terminal and `cowboy call result <call_id>`.",
  "`cowboy call capabilities` lists the preset ids (model and reasoning); omit --preset to use the session's choice.",
  "The user sees the call in Cowboy. If calls are off, Cowboy asks the user and the command waits for the answer; report a refusal instead of working around it.",
].join(" ");
