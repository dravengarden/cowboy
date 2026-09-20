# Core Code continuation finalization — 2026-09-19

Published and activated the Controller-only
[continuation finalization](../plugin-continuation-finalization.md).
Owned reads, synchronization and navigation now retain original product
approval independently of task success. Completed errors and saved outcomes
cross the same original-user authorization boundary as successful replies.
Logout, role loss and unpolled role ABA cannot disclose a failed native outcome.

The original absolute deadline is checked after authority waits and before
consuming an attempt or dispatching. Owned reads share one 60-second budget
across support, native read and HTTP observer cancellation. Expiry drops only
the Controller's finite read borrow; it neither proves remote cancellation nor
closes the native owner. Actual effects, Unknown fences and one-use mutation
records remain independent of response refusal. No automatic replay or undo
is introduced.

## Exact release

- Final clean source and connected harness:
  `093a3718d6cf549a80205b1386503164d20444a0`.
- Runtime implementation: `c9f530014187eed021edf9689ffaeaec32caf6ad`;
  its descendant changes only the test harness budget and documentation.
- Controller: `/nix/store/0c8nr1mzlbfiv9p8rzy6ymzmvkp8pd9y-cowboy-controller-release`.
- Executable: `/nix/store/fm6k4rc2k70dp85j68q3r0ajsh5k6vyy-cowboy-0.1.0/bin/cowboy`;
  SHA-256 `4c1c1abfd072bef09d9c6c8d0b135259cc4fdae8cd37b63fcd64a113b856059e`.
- Previous Controller: `/nix/store/5ikk6lz3b44aryb8b5j4412iczlzg8km-cowboy-controller-release`,
  source `576594979141d7fc8171aa1a2087d9a15902b583`.
- Activation `1789829639374709275-093a3718d6cf` committed at `2026-09-19T14:54:24.06787781Z`:
  published, succeeded, non-maintenance, no recovery. Source was pushed to
  remote main before activation.

The earlier candidate `/nix/store/x9xfc0diy4j4f4859df2y434f0m29d9x-cowboy-controller-release`
passed the expanded chain, but its activation was refused before dispatch:
the test-only follow-up was already on remote main. The final artifact was
rebuilt from that exact latest main, has byte-identical Controller ELF, and
passed the complete quality and connected gates again. The failed stale-source
submission did not restart the Controller or change its receipt. It is not a
second successful production activation.

No Plugin/SDK, Machine protocol or native binary, SQL baseline, host policy,
production role, credential or Web release changed. Private navigation admission
and the single-user permission mutation API remain closed. Later evidence/docs
commits do not change the accepted runtime artifact.

## Gates

- Complete pinned-shell `just check-compact` passed again on final source:
  1,561 main Rust tests (34 explicitly ignored), 375 standalone Machine tests
  (4 ignored), 26 core adapter tests, 126 private adapter tests (2 ignored),
  1,840 Web tests and all 18 separately isolated PostgreSQL tests. Formatting,
  lint, types, dependency, feature/build gates and all 86 structural-link
  vectors passed.
- All 76 owned-buffer tests passed. New cases cover failed support/read replies,
  synchronization/navigation failures under original credential or role loss,
  no-poll role ABA, retained Unknown/ReleaseUnknown/destination evidence,
  already-expired deadlines with no dispatch or consumed attempt, and cancelled
  reads draining across two native waits under one total deadline. The test
  clock is paused only in the source deadline test, never in the connected run.
- Source negative: the new synchronization/navigation failure regressions on
  the pre-fix `7e6cc5fe` source both return 502 instead of the required 401.
  The immutable prior Controller separately fails connected v11 at
  `owned_read_failed_reply_original_login_revocation`: its lost real read reply
  returns 502 after 40,002 ms despite product-API logout. Cleanup passed. This
  actual-process negative does not separately accept HTTP role mutations.
- The first candidate run hit the old 270-second harness cap at the final
  native budget lost-reply case, after all three new failure-authority checks
  had passed. Seven real 40-second waits require at least 280 seconds. The
  test-only cap is now 390 seconds, retaining the prior 110-second allowance
  for other work. No product timeout, fault wait or assertion was weakened;
  the incomplete run remains recorded and is not counted as accepted.
- Connected v11 passes all 31 checks in 291.51 seconds against the intermediate
  candidate and 291.63 seconds against the final artifact. Both use the
  same supplied Machine and native pair. Each holds 21 real replies, discards
  seven through normal timeouts, and verifies exact command counts, three
  connections, one deliberate cut and fixture cleanup. Original effects remain
  queryable without Apply/Execute/Open replay. Existing cancellation, uninstall,
  removed-path, connection replacement and Controller restart checks remain.
- Final clean Nix build passed 1,177 default-feature tests (18 ignored) and
  three bridge tests.
- Isolated Firefox `153.0.1` passed all 24 buffer-owner cases, fixture SHA-256
  `4e7a11c5ac7d212fa57370ad912bca000d784143febfa4d683eece9bd7b2da27`.
  This fresh-profile fixture is not physical-device acceptance.
- All six embedded Agent releases have exact signed Catalog coverage. Final
  candidate, active, next-transaction recovery and cold roles each passed two
  isolated reads of all 94 actual Catalog releases and the actual Service-owned
  host configuration preflight, before and after activation. Managed telemetry
  writer and standing background policy remain unconfigured. No durable format
  changed; historical journal matrices are not counted as newly run.

The supplied Machine is
`/nix/store/gh05dprk2zk8wqp6kjsiyn1ss30sdwb5-cowboy-machine-release`, source
`9ec6d7207f7a520085455c1e1f2a8036fcc278d8`, protocol 21. The native adapter is
`/nix/store/ng88l6jhmpnk07rxjwmz2s00l6llnwk0-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.20.0/bin/cowboy-zed-adapter`
(SHA-256 `19ab0d291ccd51d9049d17eb88659effd3ee60634ec2365e20e77d8aff8b14ec`),
and server is
`/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`
(SHA-256 `f0d700155fb4b6fc1b25ec96d33d53ecd31e2278cab93f783def78003826930c`).
Receipts also hash the supplied Machine wrapper chain, core adapter and test-only
LSP. Their temporary signed installations are not production Plugin upgrades.

## Bounded production observation

Between `2026-09-19T14:53:43.332Z` and `2026-09-19T14:55:03.665Z`, all 14
original ACP workers retain their PID, kernel start ticks and executable.
The resident Machine remains
PID `3706189`, kernel start `349734872`, generation
`worker-f07cb63df88e764816f3`. All eight Plugin installation/version/digest/
incarnation/authentication identities, Machine/Web/host profiles and all three
Victoria processes remain unchanged. No retained worker was force-replaced.

Local and public `/healthz`, `/version`, `/` and `/sw.js` return 200. SPA and
service-worker bytes match active Web files and retain `no-store`; Web version
remains `975f47157cc6ec2f36f85317960f9743`. Machine health is online/connected.
The Web release is
`/nix/store/8vq5dh4rlhiwsmjdqw71prrnw389hs3b-cowboy-web-release`, source
`7e6cc5fe9fccf877b23c9394c02de1d3f6600ad6`, independently activated at
`2026-09-19T11:57:11.290619822Z`, before this deployment window. Its historical
receipt records unpublished-at-activation; that source is now integrated into
remote main. This task preserves that Web release and does not alter its receipt
or attribute its activation to the Controller release.

## Evidence and remaining exits

Private evidence root: `/tmp/cowboy-continuation-finalization.2XgaSQSn`.
Final-artifact evidence is under `published.DpsXBB/`. These bounded receipts,
logs and observations have the following SHA-256 identities:

| Evidence | SHA-256 |
| --- | --- |
| `negative.log` | `3e27175928aac16cbc5f62ea47d263223215bffa80fd362b610e2e22a529ec0c` |
| `negative-connected-receipt.json` | `757df23af25a34574090ecbb33049235f96ae85cd31397dcae7ae9f1c045bf84` |
| `connected-receipt.json` | `73dd3cb186fbae7e132744ce84fb90b1e3a30f802c145e72a467955248b880a4` |
| `connected-final-receipt.json` | `a8abf1c55a049d0d33f00ddea2c327073a924681e5ac5a2a728283cbdd0441f8` |
| `build-published.log` | `1747556ad36f829a413c47bbf4a29ccbc19573a0624948cd69f304e461a73af5` |
| `published.DpsXBB/check-final.log` | `f607d11eeeea9369204bc0bc580e4078a3267055d3f80a7ed688075d8b6564cb` |
| `browser.log` | `0fc88c6e4c002670957f2e7939eb204a86ddbae095b996dbc7325e46d371e929` |
| `published.DpsXBB/connected-receipt.json` | `7e2b6a450b15758af5eaa88ff34be6d4c2faa6bee9d6f939c00e8d17eb05fe3e` |
| `published.DpsXBB/gate-verification.json` | `e514d0ba511366828124779f29bc3bb97a3f596ec4d061189ab4f6b2b1f21d4e` |
| `published.DpsXBB/floor-deploy/floor.json` | `f84d84b39d4ca812d399d56c40e8e7891c2e04b3941c0967272ab4fdacf29fc4` |
| `published.DpsXBB/floor-after/floor.json` | `76bdc1182dc59f2f4c269eba8853ffc5fe7f8ec099c342df6c22ccd7479d6e13` |
| `published.DpsXBB/observed-before.json` | `0958fbf0d7ef74fe605bec3840707467ae59e2f91ded4cfc11b01b1cacaafa51` |
| `published.DpsXBB/observed-after.json` | `dac3d84c903ea8657feb48fb838272ab481bfcb38a63a82fa46679b70ff5c052` |
| `activation.log` | `c87fbc00c7a6f1a0275db378e0944676c7417a72d021236cd6100250ccc629fa` |

This closes a finite original-authority/deadline gap. It does not establish an
atomic delivery fence, remote/native cancellation, principal continuity across
restart, independent post-effect restoration, general DAG/state leases,
Machine-owned Workspace/Session/security-domain identity, global retained-history
or background-effect budgets, Agent/native generation replacement, abandoned
browser recovery or supported-device acceptance. Those remain in the
[completion ledger](../plugin-refactor-completion.md#code-work-still-required).
