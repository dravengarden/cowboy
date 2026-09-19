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

Twenty-five checks cover (receipt schema `...code-buffer-connected-conformance/v9`,
requiring Machine protocol 21, Zed `1.18.0` and updated core Budget readers).
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
18. Complete original destination text is read through two native pages after
    parent release and path removal. A non-BMP scalar straddles the 64 KiB boundary;
    the original owner/content/snapshot, exact offsets and complete EOF agree.
    Historical v4 receipts do not accept this text extension.
19. A real 1,025-edit replacement is refused by the native diff bound. One
    actual Apply reply is discarded through the normal timeout; reads/release
    and retirement remain fenced until original-ID Query observes exact
    `refused/budget`. Duplicate Apply does not dispatch, original native text
    and changed disk bytes remain intact, and separate retirement/release
    complete without replay. Historical v5 does not accept this extension.
20. Actual authenticated core filesystem pages reconstruct a file whose non-BMP
    scalar crosses the 256 KiB boundary. After Machine connection replacement,
    and again after Controller restart, the original continuation returns
    `410/no-store` without an ETag or any additional Machine command, even after
    its source path is gone. The relay admits only the named fixture's typed
    file operation and a bounded page envelope. Historical v6 does not accept
    this Session-route extension; it does not prove filesystem/inode continuity.
21. Hold one actual core file reply after admission under a separate disposable
    product login. Log out that exact login through the product API, then
    release the unchanged reply. Require `401/no-store/no-ETag`, no retry or
    extra dispatch for the revoked request, and continued reading by the
    independent original login. Historical v7 does not accept this original
    credential continuation; no production cookie or database edit is used.
22. Legacy language diagnostics: query the already opened fixture, hold the next
    real native reply, revoke only its disposable product login and require
    `401/no-store/no-ETag`. Refuse further dispatch by that login; the independent
    login remains usable, without reopen, release, reload or retry.
23. Repeat the same exact authority/command-count checks for legacy hover.
24. Repeat them for legacy navigation; response refusal is not evidence that
    native destination acquisitions have been undone.
25. Repeat them for legacy outline. The relay admits only the named pre-opened
    fixture for these four query kinds; arbitrary paths and native mutations
    remain refused. Historical v8 does not accept this extension.

The connection-replacement/restart checks deliberately leave unresolved native
ownership. Teardown kills
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
intended native destination views, Machine maintenance and signed Code release
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
Its [candidate acceptance](releases/service-navigation-candidate-2026-09-17.md)
records two complete v4 runs and the exact immutable inputs. The distinct inert
sync used for reconnect testing is created only after the navigation handoff
and parent release have completed; its process-wide guard is never bypassed.
The [native text reader](plugin-native-text-reads.md) adds check 18, a new
core-only support probe and independent read validation without changing
navigation admission or enabling its Web consumer.
The [Budget outcome acceptance](releases/sync-budget-outcomes-2026-09-19.md)
adds the complete v6 19-check run and separately records Controller/Web reader
activation without Machine maintenance or production Plugin installation.
The later [signed Zed rollout](releases/zed-budget-rollout-2026-09-19.md) repeats
the 19-check v6 chain against its exact supplied artifacts, then separately
records actual Machine maintenance and Zed `1.18.0` installation. Its retained
worker observations do not establish native resume or supported-device acceptance.
The [Session read-route rollout](releases/plugin-session-read-routes-2026-09-19.md)
adds v7 check 20, rejects a supplied pre-fix Controller and accepts two corrected
immutable Controllers. It separately records Controller-only activation with
the resident Machine, native installation and original workers unchanged.
The [read-authority rollout](releases/plugin-code-read-authority-2026-09-19.md)
adds v8 check 21, rejects the prior Controller's post-logout HTTP 200 and accepts
the corrected immutable Controller's full chain. Its component activation and
later independent Provider-auth rotation are recorded separately.
The [legacy-language rollout](releases/plugin-language-read-authority-2026-09-19.md)
adds v9 checks 22–25, rejects the old artifact's post-logout language reply and
accepts the corrected immutable Controller's complete 25-check chain.
