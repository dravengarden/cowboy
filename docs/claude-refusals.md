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

## Follow-up evidence

The Controller and Web release from `15970ec0` were activated at 03:27 UTC.
Three more refusals followed in the same two conversations: request
`req_011CfC6X1Sd8HeDUUTjjWxf9` on Fable 5.1 and requests
`req_011CfC6ZciURXxHbEM1kX4n6` and `req_011CfC6aUVtRB5jxbwoJDjX3` on
Opus 5. All used the same category; no other conversation was refused. The
metadata-only analysis below counts block types, tool names, sizes, and attachment
kinds without reading conversation prose or thinking.

- **Refusals persist within a conversation.** After the first refusal, 11 of
  12 later model requests were refused. The exception was the request made
  when the old Controller drained queued work. No explicit follow-up recovered
  either conversation, even hours later or after a model change. This differs
  from issue 88364, where most conversations recovered after one turn.
- **The batching reminder does not explain these refusals.** Six of the 32
  retained Fable 5/Opus 5 conversations contain `batching_reminder_sent`, but
  only one was refused. The diagnostic conversation was refused without any
  reminder. Silent-turn, token-budget, remote-session, and queued-command
  attachments are also common in successful conversations.
- **No visible reasoning was reproduced.** The official definition says the
  request asks the model to reproduce its internal reasoning in response text.
  Refused responses contained signed thinking, tool calls, and at most one short
  text block. Cowboy's own guidance and preset system prompt contain no such
  instruction. The first conversation had no raw thinking blocks in tool results;
  its only match was the `agent_thought_chunk` identifier in source code. The
  diagnostic conversation read native transcripts containing signed thinking
  about 12 minutes before its first refusal. Its investigation confounds this
  exposure, so it remains only a lead for that conversation.
- **Partial output remains in the native chain.** The official guidance says
  to discard partial output from a refusal before continuing the conversation.
  Claude Code 2.1.272 executed client tool calls from four refused streaming
  responses and recorded their results. Its `parentUuid` chain for each later
  prompt contains every earlier refused thinking/tool block and the synthetic
  refusal text. Local transcripts cannot show whether the CLI removes them when
  building the next API request. They do not establish this retained output as
  the cause.

These points narrow the upstream report without identifying the classifier
trigger. Useful evidence for Anthropic is the request IDs, the persistent
per-conversation pattern, and a question about retaining refused partial
output. Recovery options that remain inside supported behavior are to start a
new session, or to use officially documented Claude Code refusal fallback
configuration once it is verified for this CLI version.

## Cowboy defect and repair

The Controller treated every successful protocol response as a completed turn,
including the structured `Refusal` stop reason. It could drain queued work on
the following idle edge. The transcript also marked outstanding tools completed
on that stop reason, despite lacking a tool result.

The Controller now holds the session in its existing recoverable failure state
before releasing the dispatch guard. Idle events and reconnect snapshots retain
that hold. The first production refusal after activation exposed one more
edge. Claude's native `session_state_changed` can remain `running` after
`session/prompt` returns. The worker then projects a trailing Busy -> Running
without a new turn. The Controller had released the hold on that Busy, and the
following Running could drain queued work. A detail-less worker Busy now
preserves the hold as well; only `TurnStarted` for an explicit prompt starts the
next turn. While the hold is active, an autonomous native stretch after a refusal
shows the held state rather than Busy. The worker and native session stay alive; an explicit send can reuse
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

The runtime regression exercises refusal, a queued task, the trailing native
Busy -> Running edge, a trailing idle event, reconnection, and an explicit next
turn for Claude and Codex. Supervisor tests
require reuse of the live worker without stop/resume commands. Web tests retain
confirmed tools and diagnostics, interrupt outstanding tools, and keep ordinary
prose on successful turns unchanged. These deterministic checks establish the
local repair; they do not establish that future model requests cannot be
refused.
