# Bootstrap guard compatibility — October 3

The portable installer previously accepted an older `--machine-binary`, copied
it over the bootstrap and generated a launcher requiring a diagnostic that the
older binary did not implement. The installation could report success and fail
on its next launch. Installation and refresh now reject that bootstrap before
copying or configuration writes. Register also checks before origin binding
and identity creation.

## Offline probe

The caller-selected bootstrap runs two diagnostic probes in an owned mode-0700
temporary directory, with a cleared environment, temporary HOME/XDG paths and
no enrollment token or real Controller. Empty state must return the exact
read-only guard report and create no state entry. A synthetic committed record
must return exit 1 with the terminal-admission refusal; the bytes and namespace
must remain unchanged. An empty-state success alone is insufficient.

Each process has a five-second deadline. Parsed stdout/stderr are limited to
4 KiB each. On a pending-process error or timeout, its own process group is
killed and its child reaped before scratch cleanup. The probe is a compatibility
check of caller-selected installation code, not a sandbox, signed reader claim
or production writer admission. It does not make installation a crash-atomic
multi-file transaction or fence independently invoked old installers.

## Verification

Pinned-shell Machine-host tests passed 468 with five ignored; all-feature unit
tests passed 1,775 with 42 ignored. Enabled integration targets, Clippy with
warnings denied, formatting and diff checks passed. Three added unit tests
cover false empty-only success, an observed timed-out process group disappearing
after kill/reap, and incompatible bootstrap refusal with existing payload,
identity and token bytes retained. Existing CLI/launcher tests now use a
candidate that actually supports the native diagnostic.

The opt-in `bootstrap_refresh_releases` matrix uses independently supplied
immutable Machine bundles. The legacy candidate is
`/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release`
at `38453c13456bddb4261572ab270354e47b2ffc0e`; the guarded candidate is
`/nix/store/ypqs95ri4mv39bqx9lhknn2zcz4yrn3k-cowboy-machine-release`
at `14cb070e0ab4e5ea5dfbdbd1287cf1b80c7b19dc`.

For both Service-specific and singleton legacy launcher names, the actual old
installer first creates a temporary installation without starting any service.
Enrollment evidence is synthetic, and a loopback fixture supplies public Service
identity for refresh. The tested installer rejects the old candidate and
preserves every tracked installation byte. The guarded candidate upgrades the
launcher while retaining identity, Machine id, origin and token. That launcher
refuses terminal state before opening any Machine store.

A negative control then invokes the independently supplied old installer. It
still replaces the guarded bootstrap/launcher even with a committed record.
The replacement is never started, and the record remains unchanged. This is
direct evidence that the older installer authority remains outside the new
guard, not completion of portable reader/recovery admission. All runs use
temporary homes and no real Controller, agent runtime or production records.

From the repository root in the pinned development shell:

```sh
COWBOY_TEST_OLD_MACHINE_RELEASE=/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release \
COWBOY_TEST_NEW_MACHINE_RELEASE=/nix/store/ypqs95ri4mv39bqx9lhknn2zcz4yrn3k-cowboy-machine-release \
cargo test --locked --all-features --test bootstrap_refresh_releases -- --ignored --nocapture
```

Set `COWBOY_TEST_INSTALLER_RELEASE` to a supplied immutable Machine release to
exercise its exact packaged installer instead of the current Cargo executable.
The target is ignored by ordinary tests because external release inputs are
required.

## Release receipt

Published source is `f1cf02dca1938636d5b435fd6fdb45fb18dc7aa1`. After adding
the final namespace-entry count check, the targeted installer unit tests and
exact source matrix passed again, along with Clippy and formatting. The clean
committed immutable outputs are:

- `/nix/store/dkyb2js19cj4q1blm5sd6hb09jvaaf5s-cowboy-machine-release`.
- `/nix/store/b8an6gl31ws1x8ijgrbapfqg8j9pkcj0-cowboy-source-boundary`.

The default-feature package required by the bundle passed 1,314 unit tests with
24 ignored and its enabled three-test integration target. The opt-in matrix
then passed using this exact packaged installer as its third supplied artifact.
Its SHA-256 is
`f65898e3a631387df07a5e9e88b28a4ac4716dc157ec4c4c10ea9f456a9bf385`.
The old and guarded candidate installer digests are retained separately in the
[machine-readable observations](../experiments/plugin-bootstrap-guard-2026-10-03.json).
Neither source-Cargo acceptance alone nor a release metadata declaration is
substituted for this packaged execution.

Machine-only transaction `1791024827245034676-f1cf02dca193` succeeded and
committed, published, without recovery at `2026-10-03T10:53:56.833751985Z`.
Its previous release is
`/nix/store/ypqs95ri4mv39bqx9lhknn2zcz4yrn3k-cowboy-machine-release`.
The active desired generation remains `worker-6ede7a91cc8b8b3402d4`.
Machine deployment-health reported connected, online and that exact generation.

Bounded observations at `2026-10-03T10:52:49.077Z` and
`2026-10-03T10:54:12.042Z` retained every original PID across 13 ACP worker
and four execution keeper units. Machine PID changed from `929157` to
`1285097`; Controller PID remained `486493`. Web profile and root-owned
reader-floor bytes were unchanged. HTTPS health/version/SPA/SW/Machine
deployment-health returned 200, HTML/SW retained `no-store`, and the separate
SPA version remained `4aae6c684d7dd1914dd7520ccfa82cb4`.

At `2026-10-03T10:53:47.330929Z`, the resident reader reported zero deleted
Sessions and `writer_enabled=false`. Its production namespace still contained
only `.lock`. No record was seeded or migrated, and new production deletion
remains volatile. This task activated no Controller, Web, host configuration,
macOS Manager, portable-device bundle or iOS release. Process samples do not
establish native resume or full worker replacement. Older installer/activator
authority, signed portable reader recovery and writer admission remain open.
