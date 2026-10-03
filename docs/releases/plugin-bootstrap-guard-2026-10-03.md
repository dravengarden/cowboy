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

Exact immutable build, packaged matrix and Machine activation observations are
recorded after the clean committed release completes its owning transaction.
Production Session deletion writing remains disabled.
