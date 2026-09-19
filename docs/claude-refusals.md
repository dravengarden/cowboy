# Claude classifier refusals

The September 19, 2026 investigation found nine `reasoning_extraction` refusals
in the retained, deduplicated top-level Claude transcripts on Hawk. Seven were
in the offline-first development conversation and two in the subsequent
diagnostic conversation. Both used Claude Code 2.1.272 through the official
Claude Agent SDK and Claude Agent ACP 0.77.0. The first observed refusal was
September 18 at 20:16:59 UTC. Other conversations had already produced many
successful responses on that CLI version; this is not proof of an upgrade
regression or a fleet-wide failure rate.

## What the evidence establishes

The native records contain `stop_reason: "refusal"`,
`apiRefusalCategory: "reasoning_extraction"`, and `model_refusal_no_fallback`.
They are model-service refusals, not Cowboy network, authentication, or process
failures. For example, request `req_011CfBxpxWagQkwusqpqUWZr` occurred during
ordinary TypeScript editing; request `req_011CfBzE6jCUUiAzKcNzG6Gb` rejected a
subsequent “continue” in the diagnostic conversation. The generic
reverse-engineering explanation is not evidence that Cowboy implements reverse
engineering.

Cowboy passes the official `claude_code` system-prompt preset with the supported
`excludeDynamicSections` option. It submits the current user content through
`session/prompt` and resumes native session state. Displayed thought chunks are
not reconstructed as new user messages. No such prompt-rewriting defect was
found in the inspected paths.

[Anthropic's refusal documentation](https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback)
defines this category and distinguishes a refusal from an HTTP error.
[The Claude help article](https://support.claude.com/en/articles/15363606)
explains that classifiers consider accumulated context, including files and tool
results, and can flag benign requests. Retrying “continue” retains that context.
Local transcripts cannot identify the classifier's exact trigger.

The transcript also contains native `batching_reminder_sent` attachments, a
suspected trigger in
[upstream issue 88364](https://github.com/anthropics/claude-code/issues/88364).
Their presence is only a lead: the last stored marker precedes the first refusal
by hours, with successful work between them. No controlled reproduction
established causality here. Do not remove signed thinking, edit native history,
or override private feature flags on that assumption.

The earlier suggestion to add `fallbackModel` was not a verified fix.
[Claude Code model configuration](https://code.claude.com/docs/en/model-config)
documents separate availability fallback chains and category-based refusal
fallback. `model_refusal_no_fallback` alone does not identify a missing setting.
No automatic model change or unsupported fallback override is introduced.

## Cowboy defect and repair

The Controller treated every successful protocol response as a completed turn,
including the structured `Refusal` stop reason. It could drain queued work on
the following idle edge. The transcript also marked outstanding tools completed
on that stop reason, despite lacking a tool result.

The Controller now holds the session in its existing recoverable failure state
before releasing the dispatch guard. Idle events and reconnect snapshots retain
that hold. The worker and native session stay alive; an explicit send can reuse
them. No refusal is inferred from assistant prose. The incident is classified as
`provider_refusal`, rather than a critical process crash.

The Web projection treats unfinished tools as interrupted on a structured
refusal, including historical events. Confirmed tools and the original provider
diagnostics remain intact. It does not offer a transport-continuation action for
refusals. This fixes Cowboy's handling, not Anthropic's classifier.

These are shared Controller/Web semantics for every Provider that returns the
same typed stop reason. Claude is adapter-backed; its native runtime continues
to own refusal and fallback policy. Existing worker and Plugin generations need
no replacement for this correction.

## Verification

The runtime regression exercises refusal, a queued task, a trailing idle event,
reconnection, and an explicit next turn for Claude and Codex. Supervisor tests
require reuse of the live worker without stop/resume commands. Web tests retain
confirmed tools and diagnostics, interrupt outstanding tools, and keep ordinary
prose on successful turns unchanged. These deterministic checks establish the
local repair; they do not establish that future model requests cannot be
refused.
