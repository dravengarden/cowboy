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
temporary signed installation is seeded using the normal Machine installer
before the immutable Machine starts. Thus this gate accepts native use and
HTTP uninstall, not connected Code installation admission. Catalog entries,
keys, data and source text exist only in fresh temporary directories. There are
no production URL, state-directory, environment or credential inputs.

The recipe creates a non-root user namespace, loopback-only network and private
PID/proc namespace. The transparent relay forwards original frames unchanged,
allows only finite Code/readiness/uninstall traffic and the exact runtime
generation handshake, and rejects Agent/authentication/runtime mutations.
Faults hold an actual correlated native reply while the client drops its HTTP
future; they never fabricate an ACK or change a production timeout. Receipts
contain bounded command counts, closed stages/failures and artifact hashes, not
frames, content, passwords, cookies, keys, environment or logs. Output is private,
atomic and create-only on both success and bounded post-setup failure.

Seven checks cover:

1. Anonymous refusal, real login/enrollment and effect-free preparation refusal
   before the ordinary manifest establishes native worktree readiness.
2. Cancelled open, duplicate pending/open observation and exactly one dispatch.
3. Three independent owners, exact Unicode UTF-8 content and UTF-16 position,
   language/symbol/hover observations and disk/native mismatch without reload.
4. Cancelled read, `202` release refusal while borrowed, explicit later release
   and continued use of another owner.
5. Genuine HTTP uninstall, source deletion/worktree rename and path-free reads
   of the original retained native owner.
6. Cancelled release, terminal original-ID observation and no duplicate effect.
7. Same-Machine reconnect cannot adopt the old connection; Controller restart
   cannot restore an old process-local resource ID or claim it released.

The last case deliberately leaves unresolved native ownership. Teardown kills
only fixture executables and removes their validated private runtime directories;
the PID namespace is an exceptional-path process safety net. **That forced test
cleanup is not product cleanup, rollback or recovery evidence.** No byte is
reloaded into a dirty/shared buffer. Plaintext has no LSP, so empty native hover
is dispatch/ownership evidence, not nonempty language intelligence or fresh
atomic diagnostics.

Still separate: actual Review orchestration, explicit content synchronization,
navigation destination ownership, Machine maintenance and signed Code release
publication/installation, supported devices, abandoned-browser/restart recovery,
independent restoration and general graph/state leases. This test-only change
does not require or authorize a production component restart.
