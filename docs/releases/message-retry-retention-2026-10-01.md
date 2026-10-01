# Message retry retention

## Reproduced defect

The Controller deduplicated a repeated submit using `dispatched_cmids`, then
unconditionally emitted a queue confirmation. Dispatch admission is not a user
echo: at that point neither the transcript nor the queue necessarily contains
the prompt. The Web queue replica accepted that confirmation, retired its durable
outbox entry and removed the optimistic bubble. Thus retry could erase text and
attachments while correctly preventing duplicate execution.

Confirmations now require an authoritative queue/draft row or a tagged user
message in the transcript (or its retained echo-confirmed dispatch marker).
The existing bounded dispatch window retains this distinction after the hot
transcript evicts an echo. Native review caught that long-session boundary;
the regression now exercises actual hot-log trimming before another retry.
Merely dispatched IDs and tagged diagnostic events
remain deduplication witnesses, but cannot acknowledge delivery. The transcript
timeout says delivery is unconfirmed and the message remains available, rather
than claiming a proven network failure.

## Verification

The dispatch-before-echo regression failed against the original implementation
and passed after the fix. Additional coverage checks image payload preservation
through transport requeue, repeated submit/force-submit without duplicate
execution, and confirmation after a real echo. The Web replica test preserves
text, attachment bytes and the original correlation ID through an unconfirmed
snapshot, retry and reload. Existing draft-replay coverage also passes.

The isolated Firefox product-store fixture passed cold, eight warm and eight
reconnect sends (17 deliveries per run, repeated twice). It used actual
IndexedDB and WebSocket with synthetic accounts/content and loopback-only
networking. These results are not production latency measurements or a physical
iPhone acceptance claim.

`nix develop -c just check-compact` passed, including Rust/feature-slice tests,
Clippy, 1,966 Web tests, typecheck, lint, isolated PostgreSQL regressions and
release builds. The standalone Web test invocation initially omitted the
repository's Deno test flags and hit import-map type-resolution errors; the
documented test task and separate TypeScript gate passed.
The full gate passed again after the hot-log review repair. A second native
review accepted the delivery logic and identified the required PWA version bump;
`cowboy-v1782` then passed the worker-shell tests, Web typecheck, lint and build.

Official `cowboy serve-acp` used the already authorized client and permanent OVH
Machine, with no credential copying. It exercised the same Controller submit
handler against existing idle acceptance sessions:

- Claude Code `sess-1790828728476`: text response in 4.142 seconds; valid synthetic
  64-by-64 PNG plus text response in 2.240 seconds. Both returned the requested
  marker. Streaming split the marker across chunks, so per-chunk substring
  checks were insufficient; concatenated output verified it.
- An earlier one-pixel image fixture was rejected by the Provider and removed
  from its context; this is not counted as successful image acceptance.
- Grok `sess-1790767522144`: submission reached the Provider in 2.817 seconds,
  then returned HTTP 429 `subscription:free-usage-exhausted`, reporting
  510172/500000 tokens. This independently blocks Grok completion and must not
  be described as a network failure.
- Creating another OVH session returned 409 (draining or at capacity). Existing
  sessions were retained; no capacity limit was raised or user session stopped.

The screenshot's image question was absent from the Grok transcript replay.
The original browser's exact first-send failure cannot be conclusively traced
from the retained evidence. Controller/Machine journals show disconnects, and
local browser telemetry includes an unconfirmed transcript submission shortly
after the 06:39 UTC Machine disconnect. This correlation does not establish
that it was the screenshot's message or identify the underlying network cause.
No historical lost attachment has been reconstructed. Physical-browser retry
acceptance remains separate from the CLI and deterministic regressions.

## Release boundary

Only Controller and Web need activation. The permanent OVH Machine, workers,
Provider installations, credentials and SSH/network policies are unchanged.
The component activator owns rollback and deployment receipts. Full gate and
production activation evidence are recorded after completion.
