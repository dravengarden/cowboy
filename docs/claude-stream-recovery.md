# Recovering after a partial Claude response

Claude Code can emit a synthetic assistant message followed by an ACP prompt
error after an upstream response stream closes. The `server_error` category in
that message does not identify an HTTP status or establish which network hop
closed the connection. A browser reconnect is a separate transport observation.

Claude Code Plugin 3.1.23 adds a package-owned error rule for the upstream
partial-response diagnostics. When the ACP process remains alive, Cowboy ends
the failed turn while keeping that process and its native session available for
the user's next message. It does not automatically resend a prompt that may have
already executed tools. Authentication errors, permission failures, unknown
connection errors and actual process exits retain their existing handling.

The transcript treats unfinished tools as interrupted after a failed or
cancelled turn, or an interrupted/crashed lifecycle. Confirmed tool results
remain unchanged; a later actual result can still settle an interrupted card.
The UI merges an exact standalone diagnostic with the adjacent structured ACP
failure and retains the original detail under an expandable diagnostic. Message
IDs keep earlier answer text separate. This is a display projection; persisted
events are unchanged, so the same correction applies to historical transcripts.

A retained history containing only an assistant diagnostic, with no terminal
event, is not sufficient evidence to infer that the process stopped. This
change does not synthesize completion from message text or rewrite such history.

## Verification boundaries

`src/acp_session_conformance_tests.rs` drives the production ACP client/session
loop over a real in-memory JSON-RPC connection. It sends a completed tool, an
unfinished tool and the observed partial-stream error, then sends a second user
message. The assertions require one native-session allocation, no crashed
lifecycle, no automatic prompt replay, a retained confirmed tool result and a
successful continuation. The policy test also exercises no-visible-update and
unrelated-error boundaries.

`web/src/streamFailure.test.ts` covers historical and current projections,
duplicate diagnostics, ordinary prose, cancellation, process loss, late tool
results and retained render identities. These are deterministic fixtures, not
evidence that a production network connection will never fail. The change does
not modify upstream retry limits, network routing or provider credentials.

The Web bundle and the signed Claude Code Plugin are independent releases.
An existing session keeps its exact Plugin generation until a supported,
idle-session transition selects another release. Publishing the package or
refreshing the Web app alone does not change that session's recovery rule.
