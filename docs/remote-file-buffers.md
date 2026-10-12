# Remote file buffers

A remote Claude session runs on one Device (for example OVH) while its files
live on the target (for example Hawk). Before file buffers, every Edit or
Write made four executor round trips and moved the whole file both ways:
`fs/getMetadata` and `fs/readFile` to recheck the read stamp, then
`fs/createDirectory` and `fs/writeFile` with the whole result.

With file buffers, the session keeps a verified copy of each file it reads
or writes. An Edit or Write then sends only the changed bytes to the target
as a compare-and-swap. The target applies them only if the file still has the
digest the copy was taken from, and replies with the digest it published.
Correctness never depends on the copy: the target's bytes always decide.

## Prior art

The design borrows from tools whose safety record is well established:

- **HTTP `If-Match`/ETag** gives conditional writes. Every apply names the
  base digest it expects, and a mismatch is a conflict, never a merge.
- **Mutagen** stages each change into a temporary file in the same
  directory and rechecks the target just before an atomic rename. If
  anything changed meanwhile, it does not publish.
- **Unison and Syncthing** keep a per-side record of the last agreed content
  and treat any divergence from it as a conflict. They write through
  temp + fsync + rename.
- **rsync** sends deltas against a base both sides can verify by checksum.
  The base here is one known digest, so the delta reduces to a single verified
  splice (common prefix and suffix kept). A rolling-checksum search is not
  needed.

Three-way merging (Mutagen's ancestor-based reconciliation) is deliberately
out of scope. An agent edit over a changed file must be refused and the file
read again, exactly as native Claude refuses it.

## Protocol

1. **Read.** A whole-file Read stores the bytes in the session's cache,
   named by their SHA-256. The read stamp in the tool state stays what it
   always was: the digest.
2. **Edit or Write.** If the cache holds bytes whose digest matches the read
   stamp, the edit is computed from them without reading the target. Each
   copy is rehashed on load; a corrupt or missing copy reads the target
   instead.
3. **Apply.** The session runs the target's
   `cowboy-execution-host file apply` through the executor's ordinary
   `process/start`, so it adds no new RPC and grants no new authority. The
   request carries:
   - `base`: the copy's digest, or `absent` for a new file;
   - `expect`: the digest of the result;
   - one splice: offset, deleted length, and base64 insertion.

   A request up to 48 KiB travels in argv. A larger one goes in a private
   one-shot payload file under `~/.cache/cowboy/file-buffers/`, which the
   helper deletes when it reads it.
4. **On the target** the helper:
   1. resolves symlinks, so the target of a link is written as a native write
      would write it;
   2. creates missing parent directories;
   3. takes an exclusive `flock` on the parent directory, which serializes
      concurrent Cowboy writers;
   4. reads the file and refuses anything but a regular file of at most
      4 MiB;
   5. returns `applied` at once if the file already hashes to `expect`, so
      a retried request whose first attempt published is idempotent;
   6. returns `conflict` with the current digest if the file does not hash
      to `base`;
   7. applies the splice and refuses the request unless the result hashes
      to `expect`;
   8. writes `.cowboy-buffer-<128-bit random>.tmp` in the same directory with
      the original mode bits and, when possible, the original owner, then
      fsyncs it;
   9. rechecks the file's device, inode, size, mtime and ctime just before
      publishing. Any change discards the temporary file and returns
      `conflict`;
   10. publishes. A new file is published by hard link, which never replaces
       a file another writer created meanwhile. An existing file is published
       by rename. The directory is then fsynced;
   11. reads the published file back and returns its digest.
5. **The session** records the published digest as the new read stamp and
   stores the result in its cache, so it is the base for the next edit.

### Unknown outcomes

The helper never leaves the file anywhere other than its base or its
result. If the reply is lost, or the helper fails or is too old, the session
re-reads the target and decides by digest:

- the result digest: the write happened, and it is not repeated;
- the base digest: the write did not happen, and it is done once through the
  executor's whole-file write;
- anything else: refused with native's "File has been modified since read"
  error.

If a helper that lost its reply is still running, its just-in-time check
sees the whole-file write and does not publish. A helper without `apply`
(exit status 2, before acting) disables buffers for the rest of the process.

## What keeps whole-file transfers

These files keep native whole-file reads and writes, and no copy of them is
stored on the Device:

- files matching `file_buffers.excluded`. By default these are secrets (`.env`,
  keys, `~/.ssh`, `~/.gnupg`), binary media and PDFs, notebooks (NotebookEdit
  keeps its own path), and live databases;
- files larger than `file_buffers.max_file_size`, whether before or after the
  edit;
- every file when `file_buffers.enabled = false`;
- read-only (managed child) sessions and targets without Cowboy's file
  helper.

A range Read (files of 128 KiB or more) transfers only the selected lines and
stores no copy. The first Edit of such a file reads it once, then sends only
a splice.

## Cleanup

**On the Device**, each session's copies live in
`<session>/.cowboy-claude-*/file-buffers/`. The directory is private (0700),
each file is 0600, and each copy is named by its digest.

- Each session start, and a store at most every 30 seconds, sweeps the
  directory. It removes copies unused for `file_buffers.cache_max_age`, then
  the least recently used copies beyond `file_buffers.cache_max_bytes`, and
  the temporary files of writers that have exited.
- Disabling buffers removes every copy at the next sweep.
- Deleting the session removes the directory with it.

**On the target**, each apply removes the helper's own leftovers older than
`file_buffers.target_temp_max_age` (at least 10 minutes):

- `.cowboy-buffer-*.tmp` files in the directory it writes, which an
  interrupted apply left behind;
- `payload-*.json` files in the payload directory.

It removes only regular files whose names match exactly. A temporary file is
always created and published under the directory lock, so a live apply's file
is never mistaken for a stale one.

## Configuration

All settings are Device settings in `cowboy-device.toml` and apply live.
The Machine writes them to `<state>/file-buffers.json`, the worker names that
file in the session's execution descriptor, and the session re-reads it when
it changes. An older Machine names no file, and the session uses the defaults.
A named file that is missing or invalid disables buffers rather than guessing.

| Key | Default | Meaning |
| --- | --- | --- |
| `file_buffers.enabled` | `true` | Use buffers at all |
| `file_buffers.max_file_size` | `"2MiB"` | Larger files stay whole (at most 4 MiB) |
| `file_buffers.excluded` | secrets, media, notebooks, databases | Globs that stay whole and uncached |
| `file_buffers.cache_max_bytes` | `"64MiB"` | Copies kept per session |
| `file_buffers.cache_max_age` | `"1d"` | Unused copies are deleted after this |
| `file_buffers.target_temp_max_age` | `"1h"` | Target leftovers are deleted after this |

A pattern without `/` matches the file name. A pattern with `/` matches the
absolute target path. `**` spans directories, while `*` and `?` stay within
one. ASCII case is ignored. Setting the list replaces the defaults, so keep
any defaults you still want.

## Corner cases

| Case | Behaviour |
| --- | --- |
| External write between Read and Edit | `conflict`, file untouched, native error |
| External write during the apply | Just-in-time stamp check discards the temporary file; `conflict` |
| Another Cowboy session editing the same file | Serialized by the directory lock; the second sees `conflict` |
| File created by someone else while creating it | Hard-link publish fails; `conflict`, their file kept |
| Lost reply, retried or recovered | Target digest decides; never applied twice |
| Cancellation mid-apply | Published or not, never partial; stale temporary file swept later |
| Symlink | The link's target is replaced; the link stays |
| Hard link | Replaced by a new inode, as native's write replaces it; other names keep the old bytes |
| Mode bits | Preserved, including setuid/setgid/sticky |
| Owner differs (for example a file of another user writable by group) | Owner kept by `fchown`. If that is not permitted, the file is rewritten in place after a stamp check. |
| Path is now a directory, FIFO or device | Helper refuses; the whole-file path reports native's error |
| Parent directory missing | Created, as native creates it |
| `flock` unsupported (some network filesystems) | Helper error; whole-file path |
| Disk full or permission denied | Temporary file removed; whole-file path reports the error |
| Corrupt or evicted cached copy | Rehash fails or misses; the target is read |
| CRLF, BOM, UTF-16, binary bytes | Splices are byte-exact; encoding handling is unchanged |
| Very long file names | The temporary name is short and fixed-length |
| Policy edited while sessions run | Picked up on the next file operation |

## Measurement

`tools/claude-range-native-probe.mjs` runs the same Edit of a 40 KiB source
file against the pinned executor (0.159.3), once with whole files and once
from the buffer. The receipt is
[`experiments/claude-file-buffers-native-2026-10-12.json`](experiments/claude-file-buffers-native-2026-10-12.json).

| Path | Executor calls | JSON bytes on the wire |
| --- | --- | --- |
| Whole file | 4 (`getMetadata`, `readFile`, `createDirectory`, `writeFile`) | 96,782 |
| File buffer | 2 (`process/start`, `process/read`) in 8 of 9 runs, 3 once | 1,637 |

The helper runs in a small bash wrapper that prints the helper's one-line
result and the end line in a single write on stdout (the helper's stderr is
not output). The read that returns the result therefore also completes the
call, without another read to observe the exit. A second read happens only
when the first read returns before bash has written anything.
Over the OVH–Hawk link, at a 100–250 ms round
trip, each buffered edit saves two round trips plus the whole-file transfer in
both directions. The probe also checks that mode bits survive, that a changed
target is refused untouched, and that no temporary file is left behind.
