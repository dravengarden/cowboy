# Workspace aliases share the Machine root identity — October 3

Two valid workspace IDs can name one canonical root. Previously each alias
removed the old registry entry and minted another incarnation, while only the
last one was retained. An unchanged root therefore refused reads through the
other alias and changed identities on every advertisement.

The Machine now observes each distinct path once per advertisement and carries
the same retained incarnation for all its aliases. Its 256-handle budget counts
distinct paths; an alias of an admitted root is still published after an excess
distinct root is refused. Removal of a configured alias retains the preceding
conservative configuration-retirement rule. Controller scopes stay independent
per workspace ID. See the [contract](../plugin-workspace-configuration-identity.md).

## Immutable candidate

- Product source: `d309b088b8d1ec4e0ea188154f4502cd20ff22b1`.
- Machine: `/nix/store/ykgz2cpqxl4yb37mrdv13v4vk05i3998-cowboy-machine-release`.
- Worker generation: `worker-eed1d8105af00846771d`; protocol 24 is unchanged.
- Harness source: `809aba13`, with protocol boundary unit assertions corrected
  in `26d6ba8f`. No product source changed after the candidate build.
- Supplied Controller: `/nix/store/nh3g45vdi67vv9nxvi3nb5lnhpc497df-cowboy-controller-release`,
  source `093380432aaead4523fbd386b06c051806700847`.
- Adapter: `/nix/store/z37cw1h6m1z8rnjv7ipaxvx09ab0y3ds-cowboy-zed-adapter-1.20.4/bin/cowboy-zed-adapter`.
- Server: `/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`.

## Actual negative and positive

The new real-directory unit test fails against the old implementation at the
first equality assertion between the aliases' incarnations. Thirteen focused
root-identity/configuration tests pass with the fix, including replacement,
alias removal/re-addition without an intermediate advertisement, and capacity.

The [connected negative](../experiments/plugin-workspace-alias-negative-2026-10-03.json)
uses Hawk's original Machine artifact, source `89bf4b72`, in a disposable
authenticated fixture. Installation succeeds at protocol 24, the first alias
read succeeds, and the second returns incorrect HTTP 410 after exactly two
remote root reads. It fails at `machine_owned_root_identity`; cleanup succeeds.
An earlier historical-artifact attempt stopped at installation preflight and is
not defect evidence.

The [connected positive](../experiments/plugin-workspace-alias-connected-2026-10-03.json)
changes only the supplied Machine to the candidate. All 36 v16 checks pass in
294.25 seconds. Five connections complete configuration at protocol 24; cleanup
succeeds. Both aliases read before replacement, the replaced original root is
refused and retired, then explicit refresh restores reads through both aliases.
There are exactly five remote root reads. Existing installation, authorization,
native ownership, timeout, uninstall, reconnect and Controller-restart checks
remain in the chain. The seven real 40-second timeouts are unchanged.

## Source gate

The pinned-shell complete gate passed formatting, lint, dependencies,
composition, toolchain, feature and type checks, 1,719 main Rust tests,
425 standalone Machine tests, 31 core-adapter tests and 126 private-adapter
tests. Mainline installer integration subsequently passed its ten targeted
Machine tests. Web lint/types and all 1,993 tests pass after frozen installation;
all 22 isolated PostgreSQL tests pass.
Web, Controller and private-adapter local release builds complete successfully;
the clean immutable Machine and adapter Nix outputs also build successfully.

The first full run's 100 ms OTLP admission test timed out under compilation/test
load; its focused retry and the bounded-eight-thread suite pass. A stale
protocol-23 refusal assertion was corrected for the current relay negotiation.
The Web phase first encountered September dependencies still installed in the
worktree, then three obsolete source assertions after mainline integration.
Frozen installation and assertion updates resolve those failures. The remaining
gate phases are resumed explicitly, rather than reporting the earlier failed
`just check-compact` invocation as successful.

## Activation boundary

No production component was activated by this task. The candidate requires a
separate resident Machine maintenance transaction under `AGENTS.md`; it is not
a Controller or Web deployment. Hawk's preceding October 2 root-object and
configuration-retirement adoption is recorded [separately](plugin-root-identity-adoption-2026-10-03.md).

This fixes alias continuity for advertised roots. It does not close
Machine-owned Session worktree identity, security-domain identity, general state
leases, native-owner recovery or supported-device acceptance. The isolated
negative uses production artifacts, not production state or sessions.
