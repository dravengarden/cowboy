# Claude target range reads and bounded state

Claude's signed Provider adapter uses the bound native execution environment to
read text ranges on the target. Runtime placement and execution binding remain
unchanged. Codex retains its native environment implementation; this is a Claude
adapter improvement, not another file-sync service or network policy.

## Correctness contract

The target is the file authority. OVH never creates a mirrored source tree or a
file-content cache. A range read returns selected UTF-8 lines, original line
counts and SHA-256 of the **entire** file from the same bounded read. The
existing native result shape and whole-file read-before-write check remain.
Changes outside the selected lines also invalidate edit authority, including
after cold resume.

The helper checks regular-file type, size, file-descriptor and path identity,
nanosecond modification/change times before/after reading and exact byte count.
A changed or replaced file is refused. Invalid output, encoding, process result
or failed state storage cannot grant new edit authority. Same-file tools retain
their existing serialization; independent files and cancellation remain
independent. Lost mutation receipts never authorize replay.

Writes continue through native `fs/writeFile` after checking the current full
content against the last delivered Read. This is **not atomic compare-and-swap
against unrelated host processes**: the pinned upstream protocol exposes no
conditional-write field. A concurrent external editor can still race between
validation and write, as before. Session-owned worktrees remain the normal
isolation boundary. This release does not claim universal filesystem locking,
atomic external-writer exclusion or a distributed file mirror.

## Execution and compatibility

Startup discovers `python3` through the same target Bash probe. The helper is an
inline, signed Provider utility executed by native `process/start`; it does not
use SSH or any physical endpoint. Arguments are separate argv fields. `-I -S -B`
prevents project/PYTHONPATH module shadowing, site initialization and bytecode
creation. It imports only the standard library, writes no target files, closes
its descriptor and returns bounded output. Diagnostics contain no file contents
or tracebacks.

Targets without Python retain native full-file Read. Images, PDFs, explicitly
requested PDF pages and text selections exceeding the helper's 32 KiB payload
budget retain the previous native path and output limits. Other helper failures
are explicit; they do not silently fall back to runtime-local files. No native
executor version, Machine protocol, Provider authentication or target access
permission changes.

Files below 128 KiB retain the original two native RPCs, avoiding a utility
startup for short files. The metadata response is reused only within that one
Read, never between reads. Large-file projection pays another native process
round trip in exchange for bounded transfer; both costs are measured.

Process output reads retain the one-second bound. An experimental ten-second
wait failed the fixed five-second native cancellation budget and is not shipped.
The native probe checks quiet output and TaskStop while collection is pending;
mocked event wakeups alone are insufficient acceptance for a longer wait.

## Space ownership and cleanup

- Only whole-file hashes are stored in runtime state; source bytes are not.
- Read stamps use a 512 KiB budget. Expiration revokes edit authority and
  requires another Read; it never deletes a target file.
- Completed private utility jobs release their state records. User Bash task
  handles and uncertain/running jobs retain their cursor and identity.
- State writes use an exclusive private temporary file, sync, then rename.
  Failed writes/replacements remove only their own temporary file.
- New temporary names contain writer PID and UUID. Startup removes regular,
  private, same-owner matching files only after the writer is confirmed dead.
  Live PIDs, permission-denied PID checks, symlinks and unknown/legacy names are
  preserved. This deliberately favors safety over speculative cleanup.

Installed Plugin generations, active worker state, Git worktrees and native
conversation history retain their owning lifecycle. Never recursively delete
them to reclaim this adapter's space. Old UUID-only temporary names do not prove
ownership of a dead writer and are not swept by this change.

## Acceptance

`tools/claude-range-native-probe.mjs` verifies the exact pinned executor digest,
uses closed temporary homes and generated files, compares native full and range
reads, rejects an external change outside the selected range, and verifies a
reread followed by a write reaches the target. It also verifies a project
`json.py` cannot execute, no source copy is persisted and private jobs expire.
Its temporary executor and files are cleaned after completion or failure.

Run from the pinned shell with an absolute pinned executor and a new receipt:

```sh
nix develop -c node tools/claude-range-native-probe.mjs \
  /absolute/path/to/pinned/codex /absolute/path/to/new-receipt.json
```

The probe proves native protocol behavior, not production WSS/WAN timing. It
reports both request count and bytes: starting a utility may require an extra
output-read RPC and adds local Python startup cost. Do not describe byte savings
as a measured end-to-end latency improvement. Adapter tests also cover Unicode,
CRLF, empty/missing/changed/nonregular files, invalid helper results, storage
failure, cold resume, stamp expiration and live/dead temporary ownership.

Release through the existing signed Plugin lifecycle. New sessions use the new
generation; retain existing workers. A rollback requires an exact approved
Plugin release operation, rather than copying files into installed generations.
