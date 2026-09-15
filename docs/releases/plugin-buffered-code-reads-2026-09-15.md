# Buffered code reads — Controller release, 2026-09-15

The [code-read scope boundary](../plugin-code-read-scopes.md) now covers all
eleven existing filesystem/Git HTTP readers, including cached, conditional and
failed responses. Core remote Code requests serialize closed Rust operations
with their historical JSON shapes. This slice is published on main and active
on Hawk; it is not completion of the Plugin refactor or reversible execution.

## Source and activation

- Source: `43f5db8515ec8e3e7e8377e994c5e40ea7b890d5`, clean and published.
- Controller release:
  `/nix/store/dhzf0v3d370d6mqgpr4mza0hlvdgh3zs-cowboy-controller-release`.
- Executable:
  `/nix/store/8hc2d7d9fv3jxhb7ls0ryiy33cdal7jz-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `fccf15a899c7a62905e1cf10b41e1f12b8cce91ef934467052e72f16ddddf80b`.
- Transaction: `1789458035266914024-43f5db8515ec`, `outcome=succeeded`,
  `phase=committed`, `published=true`.
- Accepted predecessor:
  `/nix/store/7a1rjwg135k1zxf286y1b1kr55pv1h74-cowboy-controller-release`,
  source `0b2d879ef5134b68790cfba561232b44169090ca`.

The machine-owned component activator restarted only `cowboy.service`. No
signed Plugin package, public Catalog, SDK, Machine protocol, SQL migration,
persistent format, host policy, Machine/native generation or Web source changed.
The changed serializer retains all ten existing core Code operations; the
independent Code adapter and installed Zed Plugin need no update. A Controller
restart expires the in-memory diff cache as before. No data rollback or extra
recovery policy is needed for this unchanged-storage release.

## Verification

The pinned `just check-compact` passed: **1,219 Rust library tests, 285
standalone Machine tests, 1,481 Web tests and 17 isolated PostgreSQL tests**,
plus 86 real-CLI Rust/TypeScript structural-link vectors, formatting, strict
Clippy, dependency audits, independent feature/native-source/Plugin closure
checks and shipped builds. Existing ignored tests, Web lint/chunk warnings and
the transitive dependency-policy warning remain visible. The clean committed
Nix release build passed its own 914 library tests and three shim tests.

Eight tests were added. Six response-boundary tests use real Hub observations
and deterministic channels, including twelve combinations of retarget/cwd ABA/
deletion/recreation with success, conditional and error replies. A stale
observation cannot invoke the reader closure, even its synchronous setup;
stable contexts preserve cache bytes/headers and errors. Two wire tests cover
fourteen positive vectors and four invalid requests. All ten operations,
optional repository/file cursors and three diff scopes retain their JSON shape.
The targeted run passed 31 tests, including the independent adapter feature
graph and previous Session, diff and Zed scope regressions.

These are source fixtures, including real Unix socket fixtures, not actual
immutable private Code HTTP endpoints or a new native Plugin installation.
The original test draft assumed the wrong directory cache-control value; that
test expectation was corrected to preserve the existing cache policy. Its
failed log is retained separately, not presented as a pre-fix product regression.
The installation/telemetry journals and policies are unchanged. Their historical
807-role matrix was not rerun or recounted for this slice. No production account
login, Plugin install, managed Victoria cutover or compensation was attempted.

## Production observations

The bounded observation window was **15:40:09–15:41:08 +08:00** (59 seconds, not
an outage measurement). Controller PID changed from `2020262` to `2188371`, and
its running executable matched the accepted artifact. All **16 worker** PID and
start-time pairs, resident Machine and three Victoria processes were unchanged.
Machine remained online on `worker-48ad34f5c4615668b75f`, with its workspace
revision and identity hash unchanged.

Web/Machine profiles and receipts, host closure/unit hashes, cold roots and both
failed-unit sets were unchanged. The pre-existing **user** unit
`xdg-desktop-portal-gtk.service` remains failed; it was neither cleared nor
repaired. The system failed-unit set remained empty.

Local and public HTTPS checks accepted `/healthz`, `/version`, exact index,
admin, service-worker and both entry-asset bytes, including cache headers. Web
stayed at `92997a83`, SPA version `9dcf7c01602e4bf519e697e619761b03`. This
bounded process evidence does not prove a native-generation swap, private
authenticated code-reader race or physical-device behavior.

Private evidence is retained at `/tmp/cowboy-buffered-code-reads-XFT4BtD1`:
targeted and complete gates, immutable build, before/pre-dispatch/after
snapshots, activation receipt/journal and exact local/public HTTP checks.
Complete gate SHA-256:
`bcf0dd52d607342eba25512f53d0ef1fd27c140e6ee0536232f6c4b73899fa8c`.
Targeted gate SHA-256:
`8348a0d101b4f770932dbb8aeccb161735132245c6783fb6350318dac48ee896`.
Nix build SHA-256:
`bb7647ebb5976191569ece881b3d81dca25f669970c42d3d5cd06c879112aa8d`.
Activation/HTTP audit SHA-256:
`ea6c905b032595c4cd444480d04da51869cfd4d91d5733ebd61a39e34ea899e2`.

Continuous Machine-owned Workspace identity, cross-request file cursor lifetime,
state leases, independent post-effect/native recovery and supported-device /
account acceptance remain in the
[completion ledger](../plugin-refactor-completion.md). Discarding a stale reply
does not undo a dispatched read, Zed readiness operation or physical-path cache
fill, and the final observation is not an atomic HTTP-delivery fence.
