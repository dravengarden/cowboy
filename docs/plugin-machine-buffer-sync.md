# Machine-owned buffer synchronization

Protocol 20 adds a finite Machine core continuation for the
[private synchronization owner](plugin-buffer-sync-owners.md). The Zed `1.9.0`
source candidate adds its distinct ownership-support probe; its private server
and upstream dependency pins are unchanged. This is not a deployed Review
effect, a Service Operator grant, or independently authorized restoration.
The installed `1.8.0` and existing native processes are not upgraded by this
source change. Machine activation remains a separate maintenance boundary.

## Authority and routing

`CodeBufferSync` is a closed control command, separate from `AdapterRequest`.
The Controller's common outgoing boundary checks the declared Service/Machine
Site and protocol floor before enqueue, including generic send entrypoints.
The enrolled Machine connection captures a non-serializable, non-cloneable
invocation before scheduling. It binds the exact immutable request, actual
connection and a 15-second monotonic command budget. Waiting for a route lock
does not renew that budget; disconnection revokes queued admission.

Only `refresh_from_disk` is declared. Preparation accepts the original open
buffer reference and bounded exact content identity, never a replacement path,
runtime, native version, deadline or caller authorization flag. Core looks up
the retained original process, serializes with that buffer's route, checks its
actual ownership protocol and reserves its owner. The adapter's new
`bufferSyncOwnerSupport` probe requires the actual private native pair and
reports its own exclusion contract. The older `nativeSyncSupport` reply cannot
satisfy it, even though it reports native protocol 1.

The generic adapter route still refuses both private effect commands before
selecting a runtime, including an injected worktree. A support reply, read lease,
content hash, signed installation or serialized purpose is not a Machine
invocation. A future Service executor must independently capture and continuously
check its Product Operator, Session incarnation and original connection before
sending this command. No new HTTP endpoint or browser call site is added here.

## Finite operation lifetime

Machine core issues its own monotonically allocated operation ID under a random
process instance, distinct from both the read lease and private adapter ticket.
It retains the original connection owner, exact native process and desired text
identity.
Follow-up commands cannot change those inputs or resolve another installation,
even after uninstall, path removal, runtime death or connection replacement.

- Only effect-free preparations expire (30 seconds). Admission is bounded to
  256 operations, including in-flight preparations. Capacity never evicts an
  unresolved or terminal operation to admit another one. At most 64 commands
  may execute or wait for an original route concurrently.
- Apply records `unknown`, consumes its one-use budget and removes expiry
  before transport I/O. Cancellation, timeout, rejected/malformed replies and
  disconnection do not reset that budget. Duplicate Apply returns local evidence.
- Pending/unknown retain the original process and buffer exclusion. Query may
  only observe the original adapter operation. Prepared/missing/retired evidence
  after attempted Apply cannot prove no effect or authorize a retry.
- Applied must repeat the exact hash/byte length and a bounded canonical native
  version. Only terminal evidence permits the local read/release fence to clear;
  the adapter independently owns alias exclusion and its native mirror floor.
- Retirement is explicit. It cannot remove pending/unknown effects. A lost
  retirement reply is resolved by original-ID observation, not another Retire.
  Bounded terminal tombstones retain no native process or capacity permit.

An admitted native Apply is not cancelled by loss of its observer. Revocation
checked before dispatch prevents a new command; it cannot undo an already
admitted native mutation. Native atomic clean/version/file/shared-peer checks
remain necessary. Disk bytes are observed, not locked against arbitrary writers.
Process-local state loss is unavailable evidence, not rollback or restoration.

## Verification and remaining boundary

Focused gates exercise closed codecs, Site/protocol refusal, actual connection
lifetimes, delayed admission, original-process loss, exact-content replies,
one-use Apply, observer loss, unknown retention, bounded capacity, inert expiry
and interrupted retirement. Empty tagged states reject unknown fields too.
The existing signed temporary-install native gate now additionally prepares
against two actual owners (refused), releases one, continues across uninstall,
synchronizes exact text, reads through the native mirror floor and retires the
original operation. These are disposable fixtures, not production authority.

The [candidate acceptance](releases/machine-buffer-sync-candidate-2026-09-17.md)
records the complete pinned gate, immutable Machine/portable-runtime builds,
actual original-process synchronization and eight connected Code regressions.
It is not signed Plugin publication, installation or Machine activation. Before
Service/Review cutover, additionally accept
the actual Controller/Machine HTTP chain, independent Product authorization,
consumer cancellation/content changes, owned navigation destinations and
supported clients. A reconnect cannot recover this operation's old authority;
independent recovery is still required. See the
[completion ledger](plugin-refactor-completion.md).
