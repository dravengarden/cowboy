# Bootstrap probe and installation share one bundle — October 3

The portable installer previously probed the caller's Machine executable and
later reopened its original path, together with its companions, for copying.
A bundle update during the probe could therefore install different bytes from
those accepted by the guard. Install and refresh now capture the host, code
adapter and ACP worker into an owned mode-0700 temporary bundle before probing.
Both diagnostic phases and the installation copies use that capture. Register
also retains its captured bundle across identity creation and installation,
instead of probing and resolving a second source bundle. Cleanup runs on success
and error through the bundle owner's destructor.

This addresses caller-path replacement during the diagnostic. Capture itself
is sequential, not an atomic source version transaction. Candidate code remains
trusted caller-selected installation code; this does not sandbox it, freeze
external dependencies, make the three destination copies one atomic operation,
or constrain another installer independently run by the same user. Hawk's
configured administrator retains root authority; a guard in the current
activator does not constrain an independently executed older root tool.
Production Session deletion writing remains disabled.

## Acceptance

The unit regression replaces all three original payloads during the final diagnostic,
for both a fresh install and a refresh. It verifies the captured bytes are
installed, refresh preserves identity/token, and the temporary bundle disappears.
Existing incompatible-candidate and terminal-state refusal tests continue to
require that the prior installation remains unchanged.

The opt-in immutable-release matrix additionally replaces all caller-owned
payloads from a diagnostic wrapper that delegates to the actual guarded release.
Both Service-specific and singleton launchers must install the captured hashes,
preserve enrollment state, and refuse a committed terminal record. The matrix
retains the actual old installer as a negative control: it still independently
replaces its own launcher and bootstrap. No replacement runtime is started.

Pinned-shell checks passed 469 Machine-host unit tests (five ignored) and 1,776
all-feature unit tests (42 ignored), with enabled integrations and doc tests,
all-target/all-feature Clippy (`-D warnings`) and formatting. After strengthening
the replacement fixture to run in the final diagnostic, the targeted installer
tests, source installer matrix, Clippy and formatting passed again on that
fixture. This separates a successful probe followed by wrong copying from an
earlier source replacement that merely makes the second probe refuse.

The exact previous installer from
`/nix/store/dkyb2js19cj4q1blm5sd6hb09jvaaf5s-cowboy-machine-release`
(source `f1cf02dca1938636d5b435fd6fdb45fb18dc7aa1`, installer SHA-256
`f65898e3a631387df07a5e9e88b28a4ac4716dc157ec4c4c10ea9f456a9bf385`)
was run against this strengthened fixture as an expected-failure control. Its
refresh returned success, but installed the overwritten host bytes rather than
the accepted capture; the hash assertion failed. No production installation was
used. The source fix passes the same fixture.

The matrix accepts `COWBOY_TEST_INSTALLER_RELEASE` to select an exact packaged
installer, plus the independent old/new candidate artifacts:

```sh
COWBOY_TEST_OLD_MACHINE_RELEASE=/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release \
COWBOY_TEST_NEW_MACHINE_RELEASE=/nix/store/ypqs95ri4mv39bqx9lhknn2zcz4yrn3k-cowboy-machine-release \
COWBOY_TEST_INSTALLER_RELEASE=/nix/store/kmzcf9rd46kn79kk2mw914xvsvn4j618-cowboy-machine-release \
cargo test --locked --all-features --test bootstrap_refresh_releases -- --ignored --nocapture
```

Run from the repository root in its pinned Nix shell. This is not portable signed
reader admission, cross-generation recovery, writer acceptance or power-loss
evidence.

## Release receipt

Published source is `646c1a6846eb61b16d3643a512cf8b05599a63ef`. The clean
committed immutable build produced:

- `/nix/store/kmzcf9rd46kn79kk2mw914xvsvn4j618-cowboy-machine-release`.
- `/nix/store/m68375b7azglw5s2qp8k9w79sn08yx04-cowboy-source-boundary`.

The default-feature package required by this bundle passed 1,315 unit tests
with 24 ignored and its enabled three-test integration target. The exact
packaged installer passed the expanded matrix: two legacy candidate refusals,
two guarded upgrades, two retained captures after original-path replacement,
two terminal launch refusals, and two independent old-installer negative
controls. The tested installer SHA-256 is
`76be38d8b446b8a4134ba203acf02db875ebfdc6c01aff0b85b2b42a63294b3b`.

Machine-only transaction `1791027185685751410-646c1a6846eb` committed
successfully, published and without recovery at
`2026-10-03T11:33:16.330349956Z`. Its previous release is
`/nix/store/dkyb2js19cj4q1blm5sd6hb09jvaaf5s-cowboy-machine-release`.
Desired generation remained `worker-6ede7a91cc8b8b3402d4`; deployment-health
reported the same generation, connected and online.

Bounded samples at `2026-10-03T11:27:46.524Z` and
`2026-10-03T11:33:36.149Z` retained every original PID across 13 ACP workers
and four execution keepers. Machine PID changed from `1285097` to `1928418`;
Controller PID remained `486493`. Web profile and root-owned reader-floor
bytes were unchanged. HTTPS health/version/SPA/SW/deployment-health returned
200; HTML/SW retained `no-store` and the separately released SPA version was
unchanged. These samples do not establish full worker replacement or native
resume.

The resident reader reported zero deleted Sessions and `writer_enabled=false`
at `2026-10-03T11:33:05.780036Z`. Production deletion state still contained
only `.lock`; no terminal record was seeded or migrated. Newly deleted
production Sessions remain process-local. This task activated no Controller,
Web, host configuration, portable device or iOS release. Installed portable
launchers are not automatically rewritten by publishing a new Hawk bundle.

The [machine-readable acceptance](../experiments/plugin-bootstrap-snapshot-2026-10-03.json)
retains before/after receipts, native and installer digests, exact packaged
matrix identity and the expected-failure predecessor control. Independent old
installer/activator authority and production writer admission remain open.
