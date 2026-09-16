# Native coordinates without replacing buffer owners

Zed Plugin / private adapter `1.5.0` replaces base-only anchor conversion with
a passive, bounded mirror of the exact server's text CRDT. It also removes
filesystem-text conversion from native hover/navigation. The upstream remote
server remains `1.13.0`; the public Plugin capability and owned-read API do not
change. This source design is not an installation receipt.

## Dependency decision

`proto`, `clock` and `text` all use Zed commit
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, matching the existing
[pinned server release](https://github.com/zed-industries/zed/releases/tag/v1.13.0-pre).
The [upstream text crate](https://github.com/zed-industries/zed/blob/aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45/crates/text/Cargo.toml)
is compiled only into the existing GPL-isolated adapter, never Cowboy's root
Cargo workspace, Controller, Machine core or Web. No user Zed installation or
mutable editor state is consumed. The exact engine avoids reimplementing
concurrent fragment ordering and undo semantics in an approximate offset cache.

The dependency review observed a newer
[1.20.0 preview](https://github.com/zed-industries/zed/releases/tag/v1.20.0-pre),
but does not mix its text engine with the pinned 1.13 protocol/server. A server
upgrade needs its own coordinated native acceptance. Cargo's locked graph and
advisory/license/source/duplicate checks cover the adapter dependencies; this
does not certify the separately downloaded server free of all vulnerabilities.
The Agent-only dependency-audit script does not cover this Code Plugin.

`sha2` uses the same `0.11` line already selected by Zed's `rust-embed` subtree.
Only exact, semver-incompatible duplicates imposed by the pinned `util`, `proto`
and `postage` graph are exempted in the private `deny.toml`, with parent paths
documented there. The unmodified `option-ext 0.2.0` MPL-2.0 license exception is
restricted to that package. Both GNU and distributable musl Linux x86_64 targets
are audited; root licensing/dependency policy is unchanged.
The package-owned Nix recipe fetches registry archives directly from the
official immutable `static.crates.io` host with the original lockfile checksums;
this avoids a rejected API redirect without changing package identity or
depending on a publisher's local Cargo cache.

## Spatial and temporal boundaries

- Each mirror belongs to one native buffer ID. It starts from that buffer's
  shared base and consumes its initial chunks, edits and undo operations. It
  cannot issue edits, save, reopen or allocate replacement ownership.
- A second insertion-only projection uses the **same** native engine. Its
  historical rope exposes FullOffset byte boundaries including deleted text,
  so malformed UTF-8 splits are rejected before applying native operations.
  It changes visibility only for validation; it is not a second CRDT algorithm.
- Incoming versions must be bounded, known and causally closed. Operation IDs
  cannot change payload; exact duplicates neither consume history budget nor
  advance the read epoch. Missing predecessors are refused, not queued forever.
- Anchor validation checks buffer identity, bias, original insertion identity,
  UTF-8 offset boundaries and native resolvability. UTF-16 input cannot clamp
  silently, split a surrogate pair, or escape the current native content. Empty
  buffers and the final empty line are valid.
- Initial sharing must finish before reads. Reload announcements invalidate
  in-flight reads and set a required native version floor, without throwing away
  valid history; reads resume only after corresponding edits arrive.
- Each query captures the current native version and an adapter-local monotonic
  epoch. A text change, reload or state replacement during the await rejects the
  result. Language, hover, navigation and symbols all enforce that check. Owned
  and legacy reads retain the original buffer lock until the query completes.
- Diagnostics retain bounded original anchors and resolve them against current
  native content. They remain **last-observed**, not synchronous refresh or
  multi-LSP atomic-snapshot evidence. Navigation destination coordinates come
  from each destination's shared native buffer, not its filesystem contents.

Native frame limits remain independent. Mirror admission caps 1,024 buffers,
4 MiB per buffer and 32 MiB total of base + encoded operations + dense-vector
charge, 4,096 text operations per buffer, 1,024 ranges/undo targets per operation,
256 vector entries and replica IDs at most 1,023. Timestamps leave room for
passive Lamport increments. These are retention/admission budgets, not exact
heap/RSS limits: the two bounded native engines, insertion strings and metadata
also occupy memory. Diagnostics retain their separate count/string/server caps.
Capacity, invalid data or lost history stops coordinate reads without closing
the native owner. Native close drops its mirror. No automatic disk fallback,
resynchronizing reopen or effect replay is introduced.

## Acceptance and remaining work

Source fixtures compare native edits, multiple ranges, concurrent replicas,
delete/undo/redo and all Unicode boundaries against the pinned engine. Transport
fixtures reject late edits in all four read paths and resolve nonempty navigation
without a destination file. These are deliberately not production LSP evidence.
The real immutable adapter/server conformance gate additionally edits a file
after open and verifies that disk-only positions are refused while the original
native positions and buffer identity remain usable. It then exercises signed
temporary installation, retained reads after uninstall,
file/worktree disappearance, owner drain and reactivation. The `.txt` fixture
does not claim a nonempty real language-server hover or diagnostic.

An initial real-process test incorrectly expected a filesystem edit to trigger
native text history; it timed out. Tracing only the disposable fixture showed
`UpdateWorktree` and `UpdateBufferFile`, but no text operation or reload. In the
pinned server, `language::Buffer::file_updated` emits `ReloadNeeded`; the
headless route does not automatically turn this into `ReloadBuffers`. The
adapter deliberately does not add that mutation to a read. Explicit content
synchronization/reload authority remains a prerequisite for Review. Native
edit/undo acceptance here uses actual engine-generated operation fixtures,
not a claim that production disk edits already synchronize.

`openedVersion` remains the original open lower bound in the owned API; it is
not upgraded into a content certificate. The closed owned read union is still
`language | symbols`. Before moving Review, implement an explicit browser/native
content or anchor certificate and navigation destination ownership. The legacy
wire shapes remain unchanged and cannot provide this certificate by themselves.
Actual Machine/Plugin activation, supported-device acceptance and independent
post-effect recovery remain separate. No generic DAG executor, state migration,
new Plugin authority or native installation is implied by these source changes.
