# Stream cached host authentication before startup — October 4

Cached startup previously loaded the complete retained artifact into a Vec before
checking its digest and unpacked payload. A large otherwise valid signed cache
could therefore fail allocation before selection. Explicit quarantine recovery
already avoids loading oversized damaged cache bytes to classify damage, but
ordinary startup needed a streaming path too.

The verifier now opens the regular artifact with the existing no-follow,
nonblocking type checks and hashes it through a 64 KiB buffer. A mismatching
digest refuses before archive parsing. Raw host verification compares the signed
artifact digest with the regular executable's streamed digest.

For archives, the same descriptor rewinds after the first digest check. An
observing reader hashes the compressed bytes actually consumed during parsing;
after parsing it drains every remaining byte and verifies the digest again.
Decoder read-ahead is already included, and trailing bytes after tar/gzip end
remain part of the authenticated artifact. Only then does the verifier compare
the unpacked directory tree with the authenticated expected path/hash map.
Thus a changed second pass cannot authenticate merely because an earlier pass
had the expected digest. No payload is executed by this diagnostic.

Artifact and file bytes use fixed buffers. Archive path/hash metadata still
scales with entry count; this is not a constant-memory claim for arbitrary
directory cardinality or a same-user/admin mutation fence. The archive grammar,
signature/proof, publisher, first reader floor, selection pointers and executable
checks remain intact. No new artifact-size cap or portable recovery authority is
introduced. Staging and explicit recovery retain their separately documented
input snapshots and limits. The production deletion writer remains closed and
administrator sudo stays available.

## Verification and production receipt

Source gates and exact native acceptance are appended after completion. The
native test uses locally signed raw and uncompressed-gzip archive fixtures with
240 MiB payloads, both before and after retaining a floor. Only the disposable
diagnostic child has a 192 MiB address-space limit, one Tokio worker thread and
zero core size, applied through the pinned `prlimit` helper. The preceding exact
immutable release must fail allocation; the new diagnostic must succeed and
report writer false. Tampered artifacts must refuse without changing the floor
or executing publisher code. Production limits and cache are not modified.

The initial host-only candidate retained accepted worker source
`90ec4edae56349cb06b9f39f13a0086197b5356b`. Latest-main integration subsequently
changed the common rusqlite feature and worker/Code observability sources, which
correctly refuses that retained-worker guard. The new pool therefore requires
independent maintenance acceptance before advancing its exact source.


The original streaming candidate `8258662f0fc0cc70cce8b1e49005626c541298d3`
passed 1,821 all-feature and 505 Machine-host tests, both Clippy configurations,
formatting and Provider checks. Its exact immutable Machine passed all four large
raw/archive and floor/no-floor diagnostics under the 192 MiB address-space limit;
the preceding immutable release reported `Error: out of memory` in each case.
Raw artifact length is 251,658,240 bytes and archive length 251,699,434 bytes.
Tampering refused twice without floor changes or publisher execution.

Integrated source `b97c2724bea23834944ded8af98e2de6729f4256`, full candidate
`/nix/store/25iq12zllfa6hqbcacdgyc0sp7k6p6gj-cowboy-machine-release`,
passed 1,845 all-feature tests (53 explicit native/fixture ignores), 526 standalone
Machine-host tests (15 ignores), both Clippy configurations, formatting and the
complete Provider gate. The large-cache native proof passed again after integration.
All preceding portable acceptance also passed: 58 quarantine cases, 36 absent
anchor cases, 28 selection-pointer cases, 5 signed refresh cases, 9 signed startup
cases, 24 cached startup cases and the bootstrap old-package negative control.

Independent maintenance acceptance is in
[the native pool receipt](../experiments/plugin-cache-streaming-maintenance-2026-10-04.json):
Codex and Claude signed-generation coexistence/descendant drain passed on the exact
new worker; native execution passed 22 and 33 checks; real remote-session
admission/recovery passed 15 checks; connected Code passed 38 checks with cleanup
complete; exact immutable diagnostic CLI log conformance passed 11 checks. These
are disposable local fixtures and fake API/auth inputs, not production Provider
upgrades or external-target coverage. Only after that acceptance is the worker pin
advanced to this exact source, generation `worker-748825b42b4302fe26ca`.
The final host-only package must retain every one of the six accepted companion
paths/digests and the accepted native Machine/installer bytes. Production receipt
follows after final artifact verification and activation.
