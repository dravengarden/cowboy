# Machine-owned synchronization candidate — 2026-09-17

This is verified source and immutable candidate evidence, **not production
activation, signed Plugin publication or installation**. Machine protocol 20
adds a core-owned original-connection continuation. Zed `1.9.0` adds a distinct
private ownership-support probe; the installed `1.8.0` and existing Code
processes are not upgraded by this task. Service authorization and ordinary
Review remain separate, unfinished consumers.

## Exact candidates

Runtime source is clean commit `e861a43cfc5d1c3a08243aaed8c1a34939ddc507`, based
on `4361897a06535808c991494394d8996784314f3d`. Subsequent acceptance-document
commits do not change those bytes. Only Linux x86_64 is declared for this Zed
candidate. The private server `1.0.0`, its upstream pin and dependency closure
are unchanged.

| Candidate | SHA-256 |
| --- | --- |
| Zed adapter `1.9.0` ELF | `b9b5c5da9bf47e54a31bf807f6ac21387a3e9ed6a895da89b551e80016bdc5f3` |
| Private Zed server `1.0.0` ELF | `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |
| Machine release entrypoint | `0e9177ba5a6ad899dbd2a343e08f4811f9675c9ea868ac1097f31e61349b774a` |
| Machine wrapped ELF | `31ce18b60cec762fda104233888514ab7faff538715cd8da767c20a332d4918d` |

Immutable outputs:

- Adapter: `/nix/store/k67bc55f5yrd4k8nlzri62m0z6kjg5rs-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.9.0`.
- Server: `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0`.
- Machine: `/nix/store/8wz9z7zzbigikyv00wcf147c42a2fcwr-cowboy-machine-release`,
  worker generation `worker-be6ca465b5840056852f`.
- Wrapped Machine: `/nix/store/g9vl7l4imwh2rzkwf04w43c4059rkv2a-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`.

The canonical release skill's portable build checks pass: both private runtime
ELFs have no interpreter or shared-library dependency, and isolated probes pass.
The native gates install only disposable packages signed by fixture keys. There
is no production publisher signature, staged Catalog acceptance, public artifact
verification or Operator installation receipt for `1.9.0`.

## Accepted behavior

The [core contract](../plugin-machine-buffer-sync.md) separates typed
`CodeBufferSync` from generic adapter forwarding. Before scheduling, the Machine
captures its original authenticated connection and immutable request in a
non-cloneable, non-serializable invocation with a finite monotonic budget. Site
and protocol checks cannot be bypassed by the common outgoing entrypoints.
Prepared operations retain their original open owner/process and exact desired
content identity, not a path or replaceable installation lookup.

Apply records uncertainty and consumes its one-use budget before I/O. Duplicate
Apply never sends another native effect. Pending, lost or invalid replies retain
the exclusion; Query observes only the original operation. Replacement
connections cannot adopt it. Exact terminal evidence permits local reads again;
explicit original-ID retirement releases capacity. Cancellation, expiry and
runtime loss cannot be interpreted as restoration.

The private support probe checks the actual native pair and separately declares
adapter ownership exclusion. The older native-only support reply cannot enable
this command. Generic private effect forwarding remains refused, even with an
injected worktree.

## Verification

The complete pinned `CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 just check-compact`
gate passes: **1,393 main Rust, 332 standalone Machine, 26 core-adapter,
75 private-adapter, 1,613 Web and 18 isolated PostgreSQL tests**, plus formatting,
strict Clippy, dependency/feature/type checks, composition conformance and
release builds. Main/Machine/private-adapter retain 32/two/two explicit ignored
tests; applicable actual-process tests ran separately. Final-source formatting
and strict Clippy were repeated successfully. An initial gate stopped on one
new function's length lint; the helper extraction and full rerun pass.
Existing dependency/lint/chunk warnings were not suppressed.

Three distinct process gates pass:

1. The actual private-server gate covers native text propagation, edit/undo/close
   refusal, source validation, retained operation identity and lost-reply query.
2. The signed temporary-install Zed gate now exercises the source Machine core
   coordinator against the exact portable pair: two original owners refuse
   preparation; releasing one permits it; uninstall between Prepare and Apply
   does not redirect it. Exact content synchronization, native-mirror reads,
   duplicate Apply without a new effect, retirement and final release pass.
   This uses a fixture Machine connection owner, not Service HTTP authorization.
3. The supplied-process connected Code gate passes all eight existing HTTP
   groups using the immutable Machine above and Controller
   `/nix/store/sd524wjyvjlfllzr70y40rbf489r0sv0-cowboy-controller-release`
   (source `255fe2b725010ea564d94c7f80e223deb9d0a9a7`). Its receipt reports
   `accepted: true`, `cleanup: true`, no failure, three connections, 59 replies,
   four held replies and one cut. It accepts installation/open/content-bound
   read/release/uninstall and replacement refusal. It explicitly does **not**
   accept a synchronization HTTP path, Review or production authority.

`nix flake check` also passes all six executed checks. Building these outputs
does not activate them. Local raw evidence is retained under
`/tmp/cowboy-core-buffer-sync-icAoTARQ`, including `check-compact-2.log`,
`final-source-lint.log`, `runtime-build.log`, `machine-build.log`,
`native-gates-1.log`, `connected-receipt.json` and `nix-check.log`.

## Production boundary and remaining work

Read-only observations before/after the gates match byte-for-byte. Controller
PID `2479478` / monotonic start `3256019304116` and resident Machine PID
`1974549` / start `3243963932728` remain active. Their profiles still point to
Controller `sd524wjy…` and Machine `yrdndji8…`; local health returns `ok`.
No component activation, Plugin upgrade, policy change, credential change or
worker replacement was issued. This finite observation does not independently
certify every retained worker/Code process or a native-generation migration.

Still required: independently captured Service Operator/Session authority and
confirmation, ordinary Review integration and cancellation/content semantics,
owned navigation destinations, supported-client acceptance, separately
authorized Machine maintenance and exact signed Plugin rollout. Abandoned
owners, Machine restart and post-effect recovery need independent authority;
missing process-local evidence cannot grant a replay. General graph/state
leases and the other exits in the [completion ledger](../plugin-refactor-completion.md)
remain open. This task does not resolve the independent Web recovery journal,
historical persistence loss, host-security adoption or managed Victoria policy.
