# Context renewal after prompt-cache expiry

Status: implemented 2026-11-15 for Claude Code; other Providers are off by
default.

## Problem

Resuming an idle native conversation re-sends its whole context. While the
Provider prompt cache is warm, that costs a cache read (0.025x–0.1x input).
After it expires, the first request is billed as a cold cache write of every
token: 1.25x input on a five-minute cache, 2x on Claude's one-hour cache. A
300k-token Claude session idle for two hours therefore pays for about 600k
input-equivalent tokens before doing any work.

Cowboy traffic measured on 2026-11-15 (per-turn `TurnUsage` grouped by the gap
between the previous `turn_end` and the next human prompt):

| Provider | Gap | Turns | Mean cache write / uncached per turn | Miss ratio |
|---|---|---|---|---|
| claude-code | < 5 min | 2769 | 14k | 1.9% |
| claude-code | 30–60 min | 167 | 40k | 3.2% |
| claude-code | 1–3 h | 157 | 336k | 33% |
| claude-code | > 12 h | 105 | 420k | 40% |
| codex | 1–3 h | 14 | 144k uncached | 3.0% |
| codex | 3–12 h | 9 | 328k uncached | 11.6% |

Claude Code on an included subscription requests a one-hour TTL for the main
conversation (`code.claude.com/docs/en/prompt-caching`); the jump past one hour
confirms it. Codex kept most of its cache for hours (GPT-5.6+ guarantees at
least 30 minutes; OpenAI says logged-in Codex users keep it much longer), and
DeepSeek misses are cheap and already covered by its cache protection.

## Behavior

When an authored message (not a slash command, scheduler or managed call)
reaches the dispatcher and all of these hold, Cowboy renews the context first:

- renewal is enabled for the session (its override, else the owner's
  `preferences.context_rollover.enabled`, default on);
- the Provider has an idle preset (`claude-code`: 60 minutes; others: none
  unless the owner sets `idle_minutes`);
- the session has been idle (no activity event) at least that long. Idle is
  measured from the last event, which never exceeds the time since the last
  request started, so a full TTL guarantees the cache is gone;
- `context_used` is at least `min_context_tokens` (default 80k). Below that the
  fresh context's own system prompt and reads would cost about as much;
- the session is dormant (`exited`, no worker or pending hibernation),
  ordinary (not system, closing, a managed child or a native fork, whose
  inherited history lives only in its native thread), has a resumable native
  id and no background work, and its context has no staged links, pending
  send or undelivered linked-context prompt. Anything else resumes normally.

A live worker idle past its policy is hibernated by a five-minute pass
(`hibernate_session`, which its Machine refuses atomically if work started),
so its next prompt takes the dormant path. Renewal never resets a running
worker: the native id is forgotten under the Supervisor lifecycle lock that
every wake path holds, after re-checking that the session is still dormant
with the same native id; if it woke meanwhile the recorded renewal is
withdrawn and the prompt resumes the old context.

Renewal keeps the Cowboy session, worktree, settings, queue and full
transcript. Under the context write lock it saves a `Rollover` record (the
boundary, idle time, context size and digest), forgets the native session id
(so the launch this prompt triggers is `session/new`) and appends a
`context_rolled_over` divider naming the same boundary. The marker is the
commit point: a record without its marker is inert. The prompt continues its
own dispatch; nothing is requeued.

Delivery is not bound to a prompt identity. At the Supervisor send boundary
every prompt (except a slash command) carries the renewal preamble while the
newest context boundary is this renewal's marker and no agent output
(message, thought or tool call) follows it. The marker is published and the
native id forgotten together under the Supervisor lifecycle lock. A retry
rebuilt from the echo drops the old renewal block, and a returned prompt is
stripped of it even after a Clear. A prompt withdrawn before the
fresh agent answered leaves it pending for the next one; a Clear writes a
newer boundary and retires it. An answer before the marker is no longer a
fork point.

The renewal preamble uses the shared linked-context prefix with a renewal
body: the boundary, why the context was renewed, a deterministic digest
(first request, the latest three human messages and the final reply of the
last turn, truncated, reminder tags neutralized) and instructions to recover
anything else on demand with `sessions.search` / `sessions.history` /
`sessions.event` bounded by `before`, or, when the answer needs much of a
long history, to hand one question to a cheap model through a context ask
(`calls.start` with `sessions`, see
[managed-agent-calls.md](managed-agent-calls.md#context-asks-over-recorded-history)).
The digest is computed once and stored,
so every attempt receives the identical preamble and requeue strips it.
No model call produces it and no history is copied.

Everything that treats Clear as the start of the native context (fork answer
lookup, Provider-version portability) also treats the renewal marker as one.
Unlike Clear, the Web keeps every earlier row; the divider is an info notice.

## Control

- Session: Session Settings → "Fresh context after cache expiry";
  `GET|PUT /api/sessions/{id}/context-rollover` (`{enabled: bool|null}`, null
  inherits); MCP `sessions.context_rollover` / `sessions.context_rollover_set`.
- Owner defaults: preferences scope `context_rollover`
  (`enabled`, `min_context_tokens` 8k–2M, `idle_minutes` map of Provider →
  1–10080 or null). Discover with `cowboy.preferences.schema()`.
- A continuation inherits the source's explicit session choice.

## Evidence: did it meet expectations?

Every judgement must come from recorded data, in two layers.

**Durable (transcript, no retention limit).** The `context_rolled_over` marker
stores `idleMs` and `contextUsed`, and every `turn_end` keeps its `TurnUsage`
(input, cache read, cache write, uncached). `tools/context-rollover-report.ts`
groups turns by Provider, idle gap before the human prompt and whether a
renewal preceded it:

```bash
nix develop -c bun tools/context-rollover-report.ts /var/lib/cowboy/cowboy.sqlite3 --days 30 [--json]
```

Expected: renewed Claude turns past one idle hour show a mean cache write near
the fresh context's size (system prompt + preamble + reads, tens of
thousands) instead of the 290k–390k cold resume baseline above, and
`fresh_ratio` falls toward the warm buckets. Later turns stay smaller because
the context is smaller.

**Controller evidence log (`cowboy::evidence`, local OTel store, ~7 days).**

| Event | When | Attributes |
|---|---|---|
| `cowboy.context_rollover.decision` | every authored prompt after ≥5 min idle | `cowboy.provider.id`, `cowboy.outcome` (`renewed`, `preferences_unavailable`, `no_preset`, `disabled`, `cache_warm`, `small_context`, `not_eligible`, `live_worker`, `context_pending`, `no_history`, `record_failed`, `raced`); a retry of the same prompt keeps its first decision, `cowboy.idle_ms`, `cowboy.context.used_tokens`, `cowboy.threshold_ms`, `cowboy.context.min_tokens` |
| `cowboy.context_rollover.first_turn` | the turn that answers that prompt | `cowboy.context.renewed`, idle, context before, `gen_ai.usage.input_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens`, `cowboy.usage.uncached_input_tokens`, `gen_ai.usage.output_tokens`, `cowboy.duration_ms` (dispatch → turn end, includes a fresh launch) |
| `cowboy.context_rollover.hibernate` | a worker idle past its cache is hibernated | `cowboy.outcome` (`requested`/`refused`), idle, context |
| `cowboy.context_rollover.digest` | a renewal is recorded | `cowboy.context.boundary_seq`, `cowboy.context.used_tokens`, `cowboy.context.preamble_bytes` (the reminder body handed over instead of the history), `cowboy.context.first_request` (`held`/`durable`/`durable_failed`/`absent`), `cowboy.context.first_request_bytes`, `cowboy.context.recent_requests`, `cowboy.context.recent_requests_bytes`, `cowboy.context.last_reply_bytes` |
| `cowboy.context_rollover.history_read` | a renewed session reads its own history, refusals included (warn) | `cowboy.operation` (`history`/`search`/`event`), `cowboy.outcome` (`ok` or the refusal code), `cowboy.bytes` returned, `cowboy.context.boundary_seq`, `cowboy.read.depth` (events behind the boundary the read starts), request `cowboy.read.limit`/`chars`/`query_chars`/`kinds` (never the query text), result `cowboy.read.items`/`scanned`/`more`/`reached_start`/`source`, `cowboy.duration_ms` |
| `cowboy.context_ask.export` | a context ask round exports its history | `cowboy.call.id`, `cowboy.ask.scopes`, `cowboy.bytes`, `cowboy.ask.records`, `cowboy.ask.omitted_scopes` (scopes cut by the export bound), `cowboy.duration_ms` |
| `cowboy.context_ask.answered` | a context ask's answer is annotated | `cowboy.call.id`, `cowboy.provider.id`, `cowboy.call.preset`, `cowboy.ask.scopes`, `cowboy.ask.stop_reason`, `cowboy.ask.answer_bytes`, input/cached/output tokens, `cowboy.ask.citations.checked`/`verified`, `cowboy.duration_ms` (call start → answer) |

```bash
cowboy logs --directory /var/lib/cowboy/logs query --from 7d --event cowboy.context_rollover.first_turn --limit 500
cowboy logs --directory /var/lib/cowboy/logs metrics --from 7d   # duration histograms per event
```

Check the reported `coverage` before treating a window as complete. Questions
these answer: how often renewal fires and why it is skipped; the first-turn
cost with and without renewal per idle bucket; how many on-demand reads and
bytes a renewal needs (a high count means the digest is too thin, zero reads
with user corrections means it may be missing context); and the latency a
fresh launch adds. The read attributes also show whether search pages stop
short (`more` with low `items`), how deep reads reach, which digest part was
missing when reads followed, refused arguments, and whether a context ask's
cost and citation accuracy justify it over direct reads. The log carries
counts and identifiers only, never prompt or transcript text.

```bash
cowboy logs --directory /var/lib/cowboy/logs query --from 7d --event cowboy.context_rollover.history_read --limit 1000
cowboy logs --directory /var/lib/cowboy/logs query --from 7d --event cowboy.context_ask.answered --limit 200
```

## Limits

- A running turn whose cache expires mid-turn (long tools, waiting for
  approval or subagents) cannot be renewed; it pays the miss.
- A preferences read failure skips renewal rather than assuming the default.
- An over-limit Claude subscription or API key uses a five-minute TTL; the
  60-minute preset then renews later than the cache expires (no worse than
  before).
- First-turn measurements are armed by the measured prompt's echo; a direct
  Prompt that overtakes it can still attribute the wrong turn's usage.
- The marker and the forgotten native id are separate write-behind records;
  the marker is written first, so a crash between them resumes the old
  context with an extra digest rather than a fresh one without it.
- The digest uses the held log tail. When that tail no longer reaches the
  current context's start, the opening request is read back from the durable
  transcript (the first human prompt after the newest Clear or renewal
  boundary); without a store, or if that read fails, it is omitted and the
  agent relies on the on-demand reads.

The local layer can be deleted when Providers resume idle conversations
without a cold full-context write, or the native CLIs perform an equivalent
lazy renewal themselves.
