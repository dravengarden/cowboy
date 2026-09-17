# Service navigation continuation candidate — 2026-09-17

The [finite Service contract](../plugin-service-buffer-navigation.md) is accepted
as source and immutable candidate evidence. It adds product Operator/Session
ownership and ordinary destination handoff to the protocol-21 Machine owner.
The private Controller admission policy still defaults `closed`. No production
Controller activation, resident Machine maintenance, Code Plugin publication or
installation, Web switch, or supported-device acceptance is claimed.

## Exact inputs

Product source: `b352cd3287ef8c1e451e9f355f191aa188862d74`.
Final clean harness and complete source gate:
`8f06817855f0298cd560becd30acbaa7cce46b82`. Descendants of the product commit
change only test ordering and relay regression vectors; they do not change the
candidate Controller's runtime behavior. The preceding Android login fixes on
remote main were integrated before the candidate was built.

- Controller: `/nix/store/d0idzxqwp45gff076y5ls49c797hj0aq-cowboy-controller-release`.
  Its exact ELF is `/nix/store/zsd21gldsh8a2szv194iassx32skb4hq-cowboy-0.1.0/bin/cowboy`;
  SHA-256 `2d4485acdf464d51fba291c877949cd22ef6d9905cf0b51975106355504ed38e`.
  Source manifest SHA-256:
  `dcf6c238a79da30738be3e60b751e13bf45d883afd58aaf3024b1cd7fb4a3358`.
- Machine: `/nix/store/3mn3lphcq80cm9r32a7shxljb4qsfkjz-cowboy-machine-release`,
  source `3b6448b28077e87b34c8dff5b24770d2f597628d`, protocol 21.
  Its entrypoint and wrapped ELF hashes remain those recorded in the
  [Machine candidate acceptance](machine-navigation-candidate-2026-09-17.md).
- Zed adapter `1.12.0`:
  `/nix/store/7xbr0gcgh02lvz7m8msrmzp800gq05hc-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.12.0/bin/cowboy-zed-adapter`,
  SHA-256 `0843d5e24232220aa8e7188b03c578f5414adf2d749b37ec958ea7a4ff95acbc`.
- Unchanged private server `1.0.0`:
  `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0/bin/cowboy-zed-server`,
  SHA-256 `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131`.

Only the affected Controller output was newly built. The Machine/native pair,
dependency pins, Plugin version, SDK contracts and SQL baselines were not changed.
The explicit test-only stdio LSP has SHA-256
`e8202d11f27f560b4ab0adf8141c236c97ddcd757c06ac21d5c811a3f6ba05f8`.
It is configured only in the temporary signed installation's private home and
is not a packaged or ambient runtime dependency.

## Accepted behavior

Service navigation IDs retain the original user, immutable Session scope,
source buffer and actual Machine connection. Product tokens and roles are
rechecked before dispatch and disclosure. Session deletion forbids acquisition
without removing independently authorized original cleanup. Unknown acquisition
or release cannot expire, replay, move to another connection or claim rollback.

Destination intent/capacity is recorded before dispatch. Original-ID Query can
recover the same ordinary buffer preparation, but cannot extend its original
TTL or substitute a path/runtime. Explicit ordinary Open owns a lifetime
independent of its parent navigation. Core snapshots expose only Service lookup
IDs and historical content/location evidence, never native references or grants.

The new closed policy codec, stored-token/role revocation cases, original-source
admission, synchronization exclusion, cancellation, capacity/expiry, strict
target validation and replacement-connection tests all pass. The canonical
release skill now requires connected receipt schema v4 and all 17 checks.

## Gates and repeated process acceptance

`nix develop -c just check-compact` passed on the final clean harness: format,
lint, dependency audit, contracts, feature boundaries, Rust/Web tests, isolated
PostgreSQL tests, and optimized builds. Counts include 1,468 full-feature core
tests, 362 standalone Machine tests, 26 standalone core-adapter tests, 98 private
adapter tests, 1,737 Web tests and 18 isolated PostgreSQL tests. The dedicated
ignored process gates below were run separately with their required inputs.

1. `zed-native-navigation-conformance`: the exact immutable pair passes the
   private native synchronization/navigation, nonempty Unicode, lost-handoff,
   original-target and independent-release cases.
2. `zed-plugin-conformance`: temporary signed installation, five nonempty kinds,
   exact original handoff across uninstall/path removal and independent read/
   release pass. This gate uses synthetic core authority, not product login.
3. `code-buffer-connected-conformance`: **two successful v4 runs**, each with
   17 checks, real password authentication, enrolled protocol-21 transport,
   actual signed installation into an empty slot and the supplied native pair.

Both connected runs observe three connections/configurations, nine held actual
replies, three deliberately discarded replies and one connection cut. The
normal 40-second transport deadlines are retained. Six navigation Executions
mean exactly one for each of five nonempty language kinds plus a separate empty
group for replacement/restart refusal. The explicit LSP audit sees each language
query exactly once. Destination preparation dispatches once; five navigation
releases mean four discarded nonempty groups and the tested parent, without
replaying a lost Release. The reconnect group is deliberately empty so it cannot
mask a broken destination lifetime by retaining the same target buffers.

After actual uninstall and completed synchronization, the HTTP destination
observer is cancelled. Query discovers the same ordinary owner; explicit Open
uses the original retained runtime. The parent Release reply is then discarded;
Query establishes Released without replay. The destination still supports an
exact-content nonempty hover and explicit release after worktree path removal.
Another navigation group refuses reconnection and is unavailable after restart.
Both fixtures report successful cleanup. Forced fixture teardown remains **test
containment**, not product recovery or native close acknowledgement.

## Audit artifacts and corrected fixture issues

Private audit directory: `/tmp/cowboy-service-navigation-HGLeWl1u`.

| Evidence | SHA-256 |
| --- | --- |
| Final `check.log` | `bc4d2c7f19d6bc0bb7015f291f0f257bc0cb0f3de0b1d7080a5399389f77e1d7` |
| Accepted `connected-3.json` | `da484357e7b5c81507865e6d29b27b60d5f47995e047a74b28fbae6bbc71a56e` |
| Accepted `connected-4.json` | `daae3972238b7b8729b102418c743ebcb6f80333ff2fd094719b184eeee07c68` |
| Static-pair `native.log` | `270cf854392aa2d93d8a1275a84c61291bfe08d0343007abb8e9a84869b3729b` |
| Signed-lifecycle `lifecycle.log` | `0ccf3c0977fb74c87059a1da40944d63528d449dfa381427ba5a273fef543339` |

Earlier attempts are not acceptance: one relay vector still expected protocol
20; one launch prepended the system Rustup ahead of the pinned Cargo and failed
the private PID-init assertion before running the fixture; and one real run
correctly refused handoff while the reconnect test's new inert synchronization
still held the process-wide native admission guard. The final harness creates
that synchronization only after handoff/parent-release completion. No native
guard or timeout was weakened. All final commands preserve pinned tool precedence;
when child tools require Nix, `/run/current-system/sw/bin` is appended to PATH.

## Still separate

Full native destination text/views, Web/native-shell consumer integration,
global native pre-allocation bounds, OS filesystem isolation, independently
authorized post-effect recovery and cross-restart restoration remain unaccepted.
The synthetic LSP does not prove production language semantics. Public cutover,
actual signed publication/installation and resident Machine maintenance require
their own evidence. General graph/state leases and the other exits in the
[completion ledger](../plugin-refactor-completion.md) remain open. This does not
report the whole Plugin refactor complete or enable production navigation.
