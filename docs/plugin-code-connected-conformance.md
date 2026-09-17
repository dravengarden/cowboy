# Connected core Code buffer acceptance

`just code-buffer-connected-conformance INPUT RECEIPT` runs the actual supplied
immutable Controller, Machine, private Zed adapter and server. It supplements
the browser owner fixtures and `zed-plugin-conformance`; neither isolated edge
test alone proves the complete authenticated HTTP/control/native chain.

From clean committed source in the pinned Linux shell:

```sh
nix develop -c just code-buffer-connected-conformance /absolute/input.json /absolute/new-receipt.json
```

The closed input has exactly `schema: 1`, `controller`, `machine`, `adapter` and
`server`. Controller/Machine values name immutable component release directories
directly under `/nix/store`. Native values name exact canonical store ELF files.
The gate hashes every executable and component provenance, uses the current
source's Zed manifest/contract, and binds those two native files into one
temporary signed release. It does not change pins or publish that release.
These are **supplied artifacts**, not assertions about active/recovery/cold host
roles. A release with no new core buffer support is expected to fail this gate.

The test reuses the core installation acceptance harness's disposable Service
identity, enrolled Machine, real password login, private SQLite state, closed
process environment and bounded HTTP client. A stopped fixture Session supplies
placement; no Agent credentials, worker or inference request is permitted. A
temporary signed release is served only as immutable fixture bytes. The Machine
starts with an empty installation slot; real authenticated Controller admission
must observe the target, stage/probe/install that release and settle both
durable journals. Catalog entries,
keys, data and source text exist only in fresh temporary directories. There are
no production URL, state-directory, environment or credential inputs.

The recipe creates a non-root user namespace, loopback-only network and private
PID/proc namespace, with an empty read-only mount hiding the host cgroup tree.
The isolated test adopts orphaned descendants, waits tracked leaders first,
and reaps descendants before accepting cleanup; Cargo as namespace init is not
itself evidence that those children were reaped.
The transparent relay forwards original frames unchanged,
allows only finite Code/readiness/installation/uninstall traffic and the exact runtime
generation handshake, and rejects Agent/authentication/runtime mutations.
Faults hold an actual correlated native reply while the client drops its HTTP
future; they never fabricate an ACK or change a production timeout. Receipts
contain bounded command counts, closed stages/failures and artifact hashes, not
frames, content, passwords, cookies, keys, environment or logs. Output is private,
atomic and create-only on both success and bounded post-setup failure.

Seventeen checks cover (receipt schema `...code-buffer-connected-conformance/v4`,
requiring Machine protocol 21 and the Zed `1.12.0` navigation-support contract).
Historical v3 receipts cover only the first eleven checks:

1. Anonymous installation refusal, actual signed Code installation, cancelled
   HTTP observation after the Machine receipt, durable completion and duplicate
   operation-ID refusal without a second install dispatch.
2. Anonymous refusal, real login/enrollment and effect-free preparation refusal
   before the ordinary manifest establishes native worktree readiness.
3. Cancelled open, duplicate pending/open observation and exactly one dispatch.
4. Three independent owners, exact Unicode UTF-8 content and UTF-16 position,
   language/symbol/hover observations and disk/native mismatch without reload.
5. Cancelled read, `202` release refusal while borrowed, explicit later release
   and continued use of another owner.
6. Genuine HTTP uninstall, source deletion/worktree rename and path-free reads
   of the original retained native owner.
7. Cancelled release, terminal original-ID observation and no duplicate effect.
8. Same-Machine reconnect cannot adopt the old connection; Controller restart
   cannot restore an old process-local resource ID or claim it released.
9. Real product authentication for explicit synchronization, two actual native
   owners refusing preparation, and Service-local read/release exclusion after
   the other owner explicitly releases.
10. Prepare before HTTP uninstall, Apply on the retained original process
    afterward, discard one actual Apply reply, and wait the normal 40-second
    transport timeout. Duplicate Apply never dispatches again; original-ID Query
    establishes the exact applied content, and original-owner content reads agree.
    The source edit preserves size and mtime to isolate explicit synchronization
    from competing native metadata/reload events. Real changed bytes, not stat
    equality, must cross the supplied native process; all conditional checks
    remain enabled. A changed native preparation is a refusal, not success.
11. Cancel HTTP retirement observation after holding its actual native reply;
    local completion and duplicate requests cannot resend retirement. A distinct
    preparation also refuses old authority after connection replacement and
    becomes unavailable, not adopted or retired, after Controller restart.

12. All five nonempty navigation kinds use actual product-authenticated Service
    admission and the enrolled connection, with complete Unicode content,
    duplicate target locations and exact UTF-16 positions.
13. Discard one actual Execute reply, wait its normal 40-second timeout, refuse
    Release of acquisition Unknown, then Query the same ID. Duplicate Execute
    never repeats the language query, as confirmed by the explicit LSP audit.
14. After uninstall and completed synchronization, cancel the HTTP observer of
    destination preparation. Query discovers the same ordinary Service resource;
    duplicate preparation does not dispatch again. Explicit ordinary Open still
    uses the retained original native runtime.
15. Discard the actual parent Release reply, wait the normal timeout, retain
    ReleaseUnknown and establish Released by original Query without replay.
16. That opened destination retains exact-content nonempty hover/read/release
    after parent release and worktree path removal.
17. A separate retained navigation group refuses connection replacement and
    becomes unavailable after Controller restart, with no command or adoption.

The last case deliberately leaves unresolved native ownership. Teardown kills
only fixture executables and removes their validated private runtime directories;
the PID namespace is an exceptional-path process safety net. **That forced test
cleanup is not product cleanup, rollback or recovery evidence.** No byte is
reloaded into a dirty/shared buffer. Plaintext has no LSP, so empty native hover
is dispatch/ownership evidence. Navigation additionally uses an explicit
repository-built, hashed test-only stdio LSP configured solely in the temporary
runtime's private home. Its synthetic answers do not accept a real language
implementation or fresh atomic diagnostics. Acquisition precedes process-wide
synchronization reservation; destination preparation follows completed sync
and never renews an expired preparation.

Still separate: actual Review orchestration, browser synchronization ownership,
full native destination views, Machine maintenance and signed Code release
publication/installation, supported devices, abandoned-browser/restart recovery,
independent restoration and general graph/state leases. This test-only change
does not require or authorize a production component restart.

The [2026-09-16 acceptance record](releases/plugin-code-connected-conformance-2026-09-16.md)
binds three successful runs to their exact supplied artifacts and records the
separate complete source gate. The later
[installation acceptance](releases/plugin-process-cleanup-2026-09-16.md) adds two
eight-group runs against the core cleanup candidates and removes the pre-seeded
installation shortcut. Neither record changes the exclusions above.
The protocol-20 [Service synchronization extension](plugin-service-buffer-sync.md)
adds the three new groups; historical v2 receipts do not accept them.
The protocol-21 [Service navigation extension](plugin-service-buffer-navigation.md)
adds checks 12–17. The isolated Controller opts into its private candidate policy;
the production default remains closed. Historical v3 receipts do not accept it.
