# Provider authentication reconciliation coordination

The Controller merges only overlapping synchronization of the same immutable
Service credential generation on the same authenticated Machine connection.
This is a Core transport mechanism, not a Plugin, login, credential cache,
durable receipt or automatic retry policy.

## Why it exists

Machine reconnect currently schedules Service-wide Provider distribution.
Concurrent reconnects, refresh propagation, stale refresh repair and failed
projection repair can therefore enqueue identical `ApplyProviderAuth` commands
on one connection. Each seal has fresh randomized encryption bytes, so comparing
the ciphertext would not identify duplicate authority. The coordinator reduces
overlapping wire requests; it does not remove all broadcast scheduling, vault
reads or resealing work.

An accepted **new** generation must still drain matching idle workers and resume
their exact native sessions, as required by CR-9. A worker PID change during
credential advancement is not by itself a defect. Coalescing same-generation
requests neither disables that rollover nor proves uninterrupted user turns.

## Identity, lifetime and failure

Each Controller `AppState` owns a bounded coordinator. A flight is identified by
the exact connection token, Provider ID and Service generation, with envelope
schema, auth-contract fingerprint, projection schema, action and Service public
key required to agree. A reused textual connection epoch is insufficient: the
connection incarnation must be identical. Only the private Service vault sealing
path supplies these envelopes. This comparison is not validation of arbitrary
untrusted envelope contents; Machine signature, monotonicity and projection
checks remain mandatory and unchanged.

The connection is captured before the asynchronous enrollment-key lookup. An
attempt cannot silently move to a replacement connection while that lookup is
pending. The first admitted caller owns one connection-bound command and its
existing 90-second deadline. Observers discard their independently sealed
envelope and await the same result/deadline. At most 128 flights and 128 observers
per flight are retained; overflow fails without sending another command.

Cancellation of an observer does not cancel the owner. Cancellation of the owner
wakes observers with an unknown outcome, removes the pending correlation and
does not resend. Timeout, disconnect and stale-connection evidence are not
converted to success. Both owner and observers recheck the connection before
returning success. A late old owner cannot erase a newer flight, and a late ACK
cannot complete a replacement operation. This does not undo a remote effect that
may already have occurred.

Completed results are not cached, including the publication/cleanup race window.
A later explicit repair really reapplies the generation: it may need to recreate
lost materialization. A different generation, Provider, Machine connection or
Service coordinator remains independent. Same-generation conflicting public
metadata is rejected rather than joined. Logout still sends its new signed wipe
generation. Aggregate distribution retains its existing Service-generation CAS;
an old completion must not mark a newer generation current.

## Evidence and limits

`server::provider_auth_sync::tests` covers duplicate admission, real shared
rejection, fresh repair after success/failure, independent authorities,
conflicting metadata, same-epoch replacement, late ACKs, cancellation, bounded
admission and cleanup. A paused-clock test joins an observer at second 89 and
requires both callers to time out at the original second 90 without a retry.

The replica test uses temporary Service/Machine identities and fake portable
credentials with the actual vault sealing, signature verifier and replica store.
It covers independently resealed duplicates, same-generation replay, generation
advance and logout wipe. It deliberately installs no Plugin: decryption,
installed-runtime materialization, worker/native-session continuity and real
credential convergence are not established by that fixture.

Production authentication, policy and credentials are not test inputs. This
change does not complete the managed telemetry cutover or the later Plugin DAG
phases, and it does not rewrite the failed all-worker continuity observation in
the [previous Controller release](releases/telemetry-policy-preflight-2026-09-14.md).
The [release acceptance](releases/provider-auth-sync-2026-09-14.md) records this
implementation's immutable gates, Controller activation and separate bounded
worker observation.
