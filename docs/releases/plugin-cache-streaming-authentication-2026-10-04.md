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

This uses `cowboy-machine-host-release` and retains the accepted worker source
`90ec4edae56349cb06b9f39f13a0086197b5356b` with all six companion paths/digests.
