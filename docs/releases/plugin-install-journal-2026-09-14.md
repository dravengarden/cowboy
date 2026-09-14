# Durable core installation release: 2026-09-14

The Service installation writer and Web source
`6d072bab04a245486834ad2d81e1b7c62affd719` are published to remote `main`
and active on Hawk. Reader `95c0e8546f179afc368b55186eb66bc63ae4dd03`
was accepted and activated first, including the separately maintained cold
recovery floor. No production Plugin was installed to verify this release.

## Delivered boundary

The [core installer](../plugin-install-journal.md) now persists exact intent
before authentication sync or installation dispatch. Each subsequent effect
requires its prior durable phase and a recheck of the original Operator,
Catalog envelope, compatibility, deadline and original connection. Restart
reconstructs interrupted installation fences before serving requests. Install
and uninstall claims exclude one another in both storage backends.

Repeated operation identities only read historical evidence. Unknown outcomes,
ambiguous commits and generic rejected ACKs do not authorize replay or an
inverse. Machine acknowledgement is persisted before post-install auth sync;
it is not a durable Machine installation receipt. Core Provider management and
the admin telemetry installer share bounded, closed, reloadable Installation
history, without replay or fence-clearing controls.

The additive PostgreSQL 0048 and SQLite 0022 migrations do not modify deployed
SQL bytes. Hawk's SQLite migration 22 is successful; its stored SHA-384 exactly
matches the release file. Production installation history was empty at the
bounded post-deployment check. That absence is not reader-compatibility proof.

## Automated acceptance

Both reader and writer passed `nix develop -c just check-compact`, including
Clippy, dependency/composition and independent-feature checks, 1,133 Rust
library tests, 272 standalone Machine tests, 1,441 Web tests, 16 isolated
PostgreSQL tests and release builds. Platform-specific ignored checks are not
claimed as production acceptance. The real Catalog at
`/var/lib/cowboy/plugin-catalog` covers all six exact embedded Agent releases.

Each immutable candidate matrix passed the following separately owned gates:

| Gate | Checks | Result |
| --- | --- | --- |
| Service install phases, corruption and duplicate observation | 72 | Accepted |
| Populated Service/Machine telemetry readers | 96 | Accepted |
| Independent telemetry writer policy | 294 | Accepted |
| Managed startup and independent local recording | 78 | Accepted |
| Connected protocol/fault flows | 45 | Accepted |
| Real disposable Victoria database pairs | 9 | Accepted |

Installation acceptance uses 12 absent/populated/corrupt/foreign-owner cases,
three Controller roles and two actual process opens. It verifies frozen
interruption evidence and the real HTTP duplicate path without dispatch. The
writer matrix includes its enabled endpoint as well as paused recovery readers.

Telemetry acceptance retains all nine cross-Site role pairs, actual fixture
login, lost ACK/disconnection without resend, bounded OTLP delivery, real
Victoria ingestion/query, database reopen and no host-restart replay. No
production credentials, private destination policy or production database
storage are used by these fixtures. Exact manifests and executable chains,
including the actual Hawk Victoria executables, were independently hashed.

Configuration-only preflight passed again immediately before writer activation
as the Service owner with the actual intended configuration. Managed writer and
background policy remain `unconfigured`; legacy selection remains explicitly
`not_checked` by this diagnostic. It is not managed telemetry cutover evidence.

## Actual recovery and release identities

| Role | Immutable release |
| --- | --- |
| Reader Controller / writer transaction recovery | `/nix/store/i3czkhfa0mg3wx480qdd89qmagbwmhcf-cowboy-controller-release` |
| Active writer Controller | `/nix/store/gz73lpnyglpp2wp582qlkyh8kxc0iazc-cowboy-controller-release` |
| Active Web | `/nix/store/ph4hmq0a59dnpss6nrd9lzsm6xnfs5fl-cowboy-web-release` |
| Cold Controller | `/nix/store/qkgjbdmbfmf6r7kp721sgrkrc5qp5dps-cowboy-controller-release` |
| Unchanged active Machine | `/nix/store/w0dszrabai56bar9zsc59m6bbqwvb5bn-cowboy-machine-release` |
| Cold Machine | `/nix/store/csdgc7n3gglrjb7084y0ccf9wh2ngmlf-cowboy-machine-bootstrap-release` |

Cold outputs are from the actual Hawk closure, not assumed equal to standalone
Cowboy outputs: followed Nix inputs change their bytes. Their source is reader
`95c0e854`. The active Machine remains `327b7f9e`; no Machine component
activation or native generation swap was performed. After writer success, the
next transaction captures the active writer as recovery; the historical
`previousRelease` still correctly records the reader used by this transaction.

Hawk's cold configuration is Columbus
`7fdfa8244790edf8b06718323cb9f876eb434003`, closure
`/nix/store/khzj60n5j9xk4d743i7ddj59a1xmrw8l-nixos-system-hawk-26.05.20260731.5b4f72e`.
The helper now has a source-owned stable package identity. A real Nix regression
test varies the host revision while requiring an unchanged helper derivation
and independent host provenance. Its actual package is identical to the
previously deployed helper. NixOS's own `nixos-version` still changes with host
provenance: dry activation identified AccountsService/polkit restarts and D-Bus
reload. These occurred in the host maintenance boundary, not a Cowboy lane.
The host receipt's empty base-unit change list does not describe those drop-in
effects; separate before/after process captures do.

Successful, published transactions are:

- Host: `1789369558083747429-7fdfa8244790`.
- Reader Controller: `1789369747922051455-95c0e8546f17`.
- Writer Controller: `1789369885636947608-6d072bab04a2`.
- Web: `1789369968649928315-6d072bab04a2`.

The actual post-host roles repeated all 96 telemetry-reader and 78 startup
checks. The post-reader active/next-recovery/cold roles match the accepted
installation matrix before writer publication and activation. Production startup
then reported `install_admission_enabled=true`, with zero fenced slots.

## Bounded production observations

Host, Controller and Web transitions preserve all 13 worker PID/start pairs,
Machine PID/start `2131179` / `3036770249940`, the Machine profile/receipt and
all three Victoria process/executable identities. Machine remains online at
`worker-240c2080a8bf9eb8968f` and reports the new host workspace revision.
The writer Controller is PID `3050186`; its live ELF matches the exact release.
There are no new failed system/user units. These are bounded continuity checks,
not proof of native resume or a generation change.

Local HTTP and public HTTPS serve exact release bytes for the SPA, admin,
their entry scripts and service worker. HTML and SW use `no-store`; hashed
scripts use immutable caching. `/version` is
`a5769a1666ecb485c5a918c98dfefcae`, the first 32 SHA-256 hex characters of
the served index, not a Git revision. SW is `cowboy-v1684`. Unauthenticated
installation-history requests return HTTP 401. Existing PWAs need a hard reload.

Private create-only proof is under `/tmp/cowboy-install-journal.8bYhHA`:

- Reader install receipt SHA-256: `0b563dcfa9b113b0165c653c93ba7dc81f077821ebe4deba59711c1521a385c5`.
- Writer install receipt SHA-256: `4b96071fbf25a5853e2deff272416635c9eecc58b0064d80708342a84bd32ad1`.
- Reader aggregate audit SHA-256: `a20884a4768f451b5770b5ab0394593394d8609e3475884efaa56ba43fd5d88d`.
- Writer aggregate audit SHA-256: `11cc67ab164fad6fd86fa9bee08eacd55526ce8ce9295880de9c59c74732c49b`.
- Settled public/runtime audit SHA-256: `6a57245898b10732a45896a76aa9725de672ee70301e7808512e765f305e47e8`.

Machine install/staging/activation receipts and CAS, independently authorized
restoration, live composition resolution, general state coexistence, real
core-security/device handoff and managed Victoria cutover remain separate
[refactor exits](../plugin-refactor-completion.md). This release closes the
Service restart gap; it does not complete the whole refactor.
