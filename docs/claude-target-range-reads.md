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

As of Plugin 3.19.6, startup discovers Cowboy's immutable, keeper-owned Rust
file utility through `COWBOY_EXECUTION_FILE_HELPER`. Native `process/start`
executes it with separate argv fields. It requires no Python, interpreter
startup or project imports. The range operation writes no target files, closes
its descriptor and returns bounded output. Diagnostics contain no file contents
or tracebacks. See the [owned utility contract](execution-connection-recovery.md#cowboy-owned-target-file-utilities).

Retained older keepers without the utility retain native full-file Read. Images, PDFs, explicitly
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
  /absolute/path/to/pinned/codex /absolute/path/to/new-receipt.json \
  /absolute/path/to/cowboy-execution-host
```

The probe proves native protocol behavior, not production WSS/WAN timing. It
reports both request count and bytes: starting a utility may require an extra
output-read RPC and adds utility process startup cost. Do not describe byte savings
as a measured end-to-end latency improvement. Adapter tests also cover Unicode,
CRLF, empty/missing/changed/nonregular files, invalid helper results, storage
failure, cold resume, stamp expiration and live/dead temporary ownership.

Release through the existing signed Plugin lifecycle. New sessions use the new
generation; retain existing workers. A rollback requires an exact approved
Plugin release operation, rather than copying files into installed generations.

## Production receipt: 2026-10-04

Signed Claude Plugin 3.4.4 is published and installed on OVH with digest
`sha256:51d13bf26ed9a9bfa5359cc08fb8da6034a178f45e501a46f2829128cefb894e`.
The installation operation is `ovh-claude-range-3-4-4-20261004`. Its first response
exceeded the 90-second deadline and fenced the slot as uncertain; exact receipt
reconciliation completed it without repeating the Machine effect. Replica and
materialization remained current. Ten observed pre-existing Machine, worker and
Claude processes retained both PID and start time. No service restart was
requested. New sessions receive the new generation; existing workers retain
their generation.

The [deployment receipt](experiments/claude-range-deployment-2026-10-04.json)
includes Linux and actual macOS runtime probes, old/new worker coexistence,
packaged native worker conformance, active/recovery/cold Controller readers,
signature-verified Catalog identity and five digest-verified public artifact
downloads. The bootstrap role was checked against the live host activation
script, rather than inferred from a remembered CLI path.

The [native measurement](experiments/claude-range-native-2026-10-04.json) reduced
serialized RPC bytes from 1,398,511 to 6,199 for the generated file and selected
range: over 99.5 percent less transfer. RPC count increased from two to five in
this sample, and local duration increased from about 14 to 26 ms. This is a
transfer improvement, not a measured WAN latency improvement. Files below
128 KiB still use two RPCs.

The [rejected wait experiment](experiments/claude-range-longwait-rejected-2026-10-04.json)
preserves the cancellation regression: approximately 9.95 seconds with a
ten-second process-read wait versus 0.958 seconds with the retained one-second
wait. The cancellation budget was not relaxed.

Rollback uses an exact signed installation operation for 3.4.3, whose retained
digest is `sha256:23eb04e31024aae477ccc71cb3111fd34f8cb21646ff99ea31ce0d3b9cd460f1`.
Observe any uncertain receipt before another operation. Do not delete either
generation while workers retain it. Production model prompts on the new
generation and end-to-end WAN timings are not claimed by these receipts.

After acceptance, this task removed its private probe extraction, build targets,
dependency installation and generated release/site/Web output: 14,831,604,120
logical bytes. Each deletion checked the exact task-owned directory and absence
of process references first. Signed Catalog copies, enrolled Machines,
installed generations, credentials and user workspaces remain intact. The
runtime cleanup rules above apply independently to future sessions.

Final integration preserves main's concurrent component history through 3.38.0
and appends 3.39.0 for the new Provider source snapshot. The signed Plugin keeps
its 3.38.0 binding: its actual SDK dependency pins are identical in both branches.
The immutable original build commit and published bytes are retained. The full
gate passed before the final additive Machine-host merge; Provider checks and
all 13 Machine-component tests passed again after integration. No Controller,
Machine-host, Web or native application release is activated by this task.

The subsequent [OVH API path observation](experiments/ovh-provider-direct-api-2026-10-04.json)
captured outbound TLS ClientHello for `chatgpt.com` on native IPv6 and
`api.anthropic.com` on native IPv4/IPv6, all on OVH's `ens3`, with outer
destinations matching official-origin DNS. Observed native Provider processes
had no HTTP/SOCKS proxy environment or custom API base URL. Kernel defaults
use `ens3`; no redirect, TPROXY or DNAT rules were present. This establishes
sampled direct API egress, not an audit of every historical request.
Account usage placement remains independent: Anthropic uses OVH, while
OpenAI, xAI and DeepSeek use Hawk as previously selected. The Claude model
probe exceeded its local response window, so successful full-turn acceptance
and the timeout's root cause remain unverified despite the direct-path evidence.
