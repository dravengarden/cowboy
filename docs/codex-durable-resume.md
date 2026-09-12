# Codex restore and durable media

## Decision

Fix resume at the Codex Provider transport boundary. Keep ACP as the integration
contract and keep native storage owned by Codex. Do not modify Cowboy's
transcript database, rewrite live native rollouts, increase the 240-second
deadline, or create a replacement native session after a failed restore.

Native media references are a separate storage-format change with explicit
reader compatibility and migration acceptance. They are desirable for disk use
and large history pages, but they are not required to resolve the observed
restore incident. This release implements the transport change. It does not
implement the media format, storage migration, automatic batching or a new
recovery UI.

## Evidence and corrected root cause

The affected rollout was 220,926,098 bytes across 3,582 lines. Its 152 completed
image-generation display events contained 195,359,860 base64 characters. All 152
saved images existed and matched the encoded contents. A native thread-history
SQLite projection contained another copy of approximately the same inline data.
Removing display results alone would leave approximately 25.57 MB, not 16 MB.

The installed Provider's exact `CODEX_PATH` bypassed the Machine-side shim that
was supposed to add `excludeTurns`. The upstream launcher also places `-c`
options before `app-server`, which defeats a shim that checks only the first
argument. Consequently the actual ACP adapter requested a complete native resume
response: one 199,406,480-byte JSONL frame.

The ACP 1.10.0 reader repeatedly concatenated chunks to a string and searched
that entire accumulated string for a newline. A single large frame caused
quadratic work. The recorded failure used about 246 CPU seconds and peaked near
3.8 GB. In isolated reproduction the original reader still had not completed
after 60 seconds. A linear reader handled the full frame in approximately 1.46
seconds, but still consumed roughly 859 MB: avoiding the frame is the primary
fix.

Native resume with `excludeTurns` and a fresh projection completed in about 1.75
seconds; a full native response took about 3.31 seconds. The packaged corrected
ACP entrypoint restored the unchanged rollout in 1.734 seconds cold and 0.486
seconds warm, returning about 9.9 KB of ACP output. These are single-host
fixture measurements, not a universal latency guarantee.

Codex 0.153.4 already has paginated thread storage and a model-context
checkpoint loader. The latter scanned roughly a 5.12 MB suffix for this thread.
It is incorrect to attribute the entire 240-second failure to mandatory native
hydrate or to assume every warm resume must deserialize the whole rollout.

## Why stripping `result` is not a supported migration

Relevant native source is pinned at `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`
(Codex 0.153.4).

- `codex-rs/ext/items/src/image_generation.rs` declares the generated image
  result as a required string. `saved_path` is an optional saved output locator.
- `codex-rs/app-server/src/request_processors/thread_processor.rs` handles
  `thread/resume`; `codex-rs/thread-store/src/local/` owns durable projection
  and context loading. `excludeTurns` avoids returning display turns; it does
  not eliminate all native restore work.
- `codex-rs/history/src/rollout_payload.rs` and `history/src/lib.rs` distinguish
  ordinary display events from `ResponseItem` and compaction replacement
  history.
- `codex-rs/core/src/session/mod.rs` restores and prepares model context.
  `codex-rs/core/src/image_preparation.rs` prepares its image inputs. A
  generated display result or `ImageView` file locator is not an implicit
  model-image reload.
- ACP `src/CodexToolCallMapper.ts` presents generated results to the client. The
  retained version does not load missing results through `savedPath`.

In fixture experiments, deleting the required field made 152 completed display
items disappear on deserialize; it did not necessarily fail the whole resume.
Using an empty string preserved item parsing but lost image presentation. An
existing projection could retain old inline data and stale rollout offsets after
a manual rewrite, producing a misleading successful resume.

The fixture had 20 model `input_image` data URLs, rather than 152. Display-only
stripping left response items and checkpoints byte-equivalent. That does not
make stripping safe: UI history is still lost, and stripping model images or
checkpoint images would change subsequent model context. Neither a path string
nor a generic `asset://` URL is a replacement for pixels: current image
preparation may omit an unsupported URL. Test actual subsequent request contents
before claiming visual context preservation.

## Durable media format

Codex should own an immutable asset store and the reference resolver. The
storage record should distinguish inline legacy data from a typed blob
reference; the public live event and persisted representation need not be the
same Rust type.

An asset reference identifies a digest, MIME type, byte length and dimensions.
`savedPath` remains an export or workspace locator, not asset identity. Before
appending a durable reference, write and synchronize the blob, atomically
publish it, then append/synchronize the rollout and update the rebuildable
projection. A missing or corrupted referenced blob must produce an explicit
error. Never silently substitute a text placeholder during model restore.

The asset store must track retention from threads, forks, checkpoints and
pending exports. Garbage collection needs leases and a grace period. A workspace
file being moved, an output directory being deleted, or an older Provider
generation being retired must not invalidate retained native history. Avoid
cross-account deduplication unless account boundaries and retention leases are
defined.

Read behavior has three separate budgets:

1. Resume restores metadata and the necessary model-context checkpoint. It does
   not return the full display history. Do not announce readiness until that
   context is usable; metadata-only acknowledgement is not a successful restore.
2. History uses bounded pages by both serialized bytes and item count. Return
   references or thumbnails; resolve full assets only when requested.
3. The next model request resolves only the images selected by the current model
   context, immediately before normal image preparation. Preserve the previous
   ordering, image detail, pruning and compaction semantics.

Lazy pages alone do not bound a cold projection rebuild. Add observability for
projection lag, checkpoint bytes, selected media bytes and restore stage timing.
Retain the existing append-only rollout and `LocalThreadStore` architecture;
there is no evidence that Cowboy needs a new conversation database or transport.

## Compatibility and native migration

Adding an optional reference beside a now-empty `result` is insufficient: an old
reader may accept the item while showing no image. Add an explicit
storage-format compatibility fence and prove that old readers reject unsupported
histories clearly, without silently dropping items or making unrelated old
threads unusable. The current `ThreadHistoryMode` discriminator is a possible
fence, not proof that all list/read/resume paths already handle a new value
safely.

Ship reader support before enabling reference writers. Keep live/wire
compatibility separate from storage compatibility. Candidate native versions
require Linux, macOS and retained-generation acceptance, including shared
authentication-home access. Do not rely only on Cowboy's Provider auth-contract
fingerprint: it does not describe the native storage format.

Reuse Codex's owned migration machinery in
`codex-rs/thread-store/src/local/rollout_migration.rs`: exclusive maintenance
and writer locks, staged transformation, projection verification, publication
journal and crash recovery. A turn-completed notification does not prove a
writer is quiescent. Migrate only with a declared compatibility/rollback policy;
preserve a recoverable original until publication and readback have succeeded.

Required acceptance before enabling writes or migrating existing threads:

- Old inline, new reference and mixed histories restore and display identical
  image bytes; a subsequent mocked model request has equivalent image contents.
- Cold/missing/stale projection, fork, compaction, migration interruption,
  missing blobs, corrupted hashes, concurrent writer and restart behavior are
  covered.
- Unknown-format old readers fail explicitly; unrelated retained-generation
  sessions continue working. Rollback does not erase or misread new-format data.
- Synthetic histories with 152, 1,000 and 10,000 images keep resume output
  bounded; disk amplification, peak memory, cold rebuild and warm latency are
  measured.

## Explicit recovery and batching

An optional **Continue in new session** action is a distinct user decision. It
creates a new Cowboy session and native thread in the same worktree. It never
copies the old native ID into a resume request. Keep the old session and queued
prompts available; do not silently replay them or mark them completed.

Build a reviewable handoff from workspace assets and known task state. Include:

- Source Cowboy session and native thread IDs for provenance only, with an
  explicit instruction not to resume or fork that native thread.
- Exact Machine/worktree, repository branch and dirty-state summary; do not
  checkout, reset or clean the shared worktree as part of continuation.
- Output directory and an asset manifest path. Each entry records the path,
  digest, dimensions, purpose, accepted/rejected state and version relationship.
- Task objective, palette/style constraints, accepted examples and rejected
  directions. Preserve filenames and naming conventions.
- Remaining work and the next requested action. Copy only user-selected queued
  requests. State uncertainty when previous conversation state is unavailable.
- Reference images to inspect, preferably a contact sheet plus selected
  originals. Paths alone do not mean the new model has seen the images.

Do not make "20-30 images" a permanent thread limit. Image count is a weak proxy
for encoded bytes, and thread rotation incurs handoff/context loss. Before the
transport fix, a 32 MiB display payload or 40 generations can be a conservative
warning, not a prediction that reopening will fail. After the fix, measure
resume stage latency and response bytes first. For native storage planning, use
encoded display bytes, projection amplification and active model-context bytes;
leave batching under explicit user control until measured budgets justify a
threshold.
