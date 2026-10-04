# Signed portable bootstrap package admission — October 4

The portable installer previously accepted only administrator-selected executable
paths. Its behavior probe and captured three-program snapshot did not authenticate
the package publisher or bind the companion programs to the Machine executable.

The installer now accepts `--bootstrap-manifest`, `--bootstrap-artifact` and
`--artifact-public-key` together. They conflict with `--machine-binary`. This
opt-in path authenticates a local singleton Machine-host component manifest using
the existing canonical v4 reader-only signature before decompressing or executing
any package code. The declared reader must be schema 1 with writer 0. The signed
SHA-256 binds an exact gzip tar package containing only the three root regular
files `cowboy-machine`, `cowboy-code-adapter` and `cowboy-acp-worker`; the entrypoint
must be `cowboy-machine`. Missing, duplicate, extra, alternate-path or special
entries refuse. All three files come from the authenticated in-memory package,
never independently selected companion paths. The unsigned artifact URL is not
fetched or used as publisher identity. Optional signed component probe metadata
retains its existing proof representation; installation executes the required
portable guard diagnostic, not arbitrary readiness arguments from that field.

Manifest JSON is closed, including component ID, reader and optional probe,
and rejects duplicate known fields. Version and generation must be nonempty.
Manifest reads are limited to 64 KiB, public key reads to 16 KiB, compressed
artifact reads to 256 MiB, total exposed payload bytes to 512 MiB and decoded tar
stream to that limit plus 64 KiB of headers. Input reads require regular files,
use no-follow/nonblocking opens, and do not create Machine state. Publisher
verification and artifact hashing precede any archive extraction or code probe.

The owned mode-0700 snapshot executes the existing bounded guard against empty,
committed-journal and reader-floor fixtures. It rechecks all authenticated payload
bytes and the exact three-entry tree after the probe and before installation
publication. A truthful but incompatible guard still refuses. A guard modifying
its authenticated companion refuses. Namespace and floor absence are rechecked
after probing, before origin/bootstrap/token/launcher publication; external probe
effects are retained, never erased to make admission pass.

This is pre-floor package installation admission. Existing floor entries still
refuse every register/install/refresh, including a valid signed package. Refresh
checks local refusal state before contacting the Controller, so an unavailable
Controller cannot obscure that offline refusal. Installing
this package does not create a component-cache anchor or a portable reader floor,
and does not enable the deletion writer or admit committed portable deletion
state. The installed bootstrap remains administrator-owned startup authority;
this change does not authenticate its bytes again before its own future execution,
fence independently authorized older installers, authorize publisher key rotation,
or provide signed recovery after active cache loss. Those require separate
selection, evidence retention and native acceptance. Signed executable probes are
not sandboxes and concurrent administrator/same-user mutation is outside this
snapshot check's authority boundary. Existing caller-selected trusted bootstrap
installation remains available only before the floor, with the same guard checks.

## Validation and activation

Source tests cover authenticated capture, manifest and companion tampering,
wrong publisher, pre-probe no-execution markers, exact archive layout, closed and
bounded metadata, snapshot mutation by a signed probe, CLI combinations and a
probe-created floor refusing before installation publication. Existing v3/v4 proof
fixtures retain their exact bytes; the shared proof implementation changes no
Machine protocol or accepted worker interface. The first source gate passed all-feature library tests (1,804 passed, 45 ignored),
integrations, standalone Machine tests (496 passed, 8 ignored), all-feature and
default Clippy with warnings denied, and Plugin/Provider checks. Main's independent
usage, execution-transport and desktop changes were then integrated; final-source
all-feature/default Clippy, Plugin checks and Machine tests (497 passed, 8 ignored)
passed. The immutable source-boundary and derived worker-registry checks passed.
The default Nix package's parallel check initially failed the existing
`hanging_probe_is_killed_and_reaped` assertion; the same source and derivation
passed an unchanged retry (1,329 passed, 26 ignored). No assertion, deadline,
signature, warning gate or check was disabled.

The exact final immutable release passed three native controls: a signed package
installed all three captured files, then refused a floor-bearing refresh offline;
the generated launcher retained all twenty-four raw/archive cache and floor cases;
and bootstrap admission accepted the current guard while refusing the preceding
cache-only guard. The first exact installer acceptance exposed Controller discovery
preceding local floor refusal; the installer now checks local committed/floor intent
before refresh discovery. The final native offline refusal verified that fix.

## Production receipt

Published source `fc58af43533a58dd0c2fbe3223a01b2c52e7d86c` was activated from
`/nix/store/q3247pa0r325gh850pj8l4gs907q0p6c-cowboy-machine-release` through the
unchanged installed component owner. Transaction
`1791097784887338397-fc58af43533a` succeeded, published and committed at
`2026-10-04T07:09:53.454930758Z`, with maintenance enabled and no recovery.
The predecessor is `nhdfwgfsz63ji462cgp264v51ihsmp7g`, the preceding portable-floor
release. The owner unit finished successfully; both component in-progress files
are absent and no failed system/user unit was observed or reset.

The exact running native executable is
`/nix/store/pn8fyh11p700m7lzrw3jbmjr0r45gvlv-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`,
SHA-256 `275f8dbc50be379562f9975c14bc01e58734a136d49634c29a4840c4a57e535e`.
The installer entrypoint SHA-256 is
`5d14fead8c8b781349af31004fb52693da90b48dd13505c850dd3b7685bc3018`.
All six worker/proxy/Code/Zed/JS companion paths and digests equal the preceding
bundle and retain source `406471a28de430debf6f8363b44abc3e621589d7`, generation
`worker-6ede7a91cc8b8b3402d4`.

Samples at `2026-10-04T07:09:06.427Z` and `2026-10-04T07:11:21.758Z` retain
all thirteen worker and five keeper IDs, states and PIDs exactly. Resident Machine
PID changed from `3171975` to `4007545`; its `/proc` executable and digest match
the accepted release. Controller PID `959309` and its component receipt stayed
unchanged. This is bounded Linux process continuity, not full native Session
resume, power-loss recovery or device acceptance.

All five public HTTPS checks returned 200; Machine health reports connected and
online on the retained generation. Web profile and SPA version stayed unchanged;
HTML/SW retained `no-store`. The root floor SHA-256
`26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017` and sudoers
SHA-256 `149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`
were preserved; noninteractive sudo remains available. At
`2026-10-04T07:09:44.971327Z`, the deletion reader logged zero IDs and
`writer_enabled=false`; the namespace still contains only `.lock`. The separate
Plugin installation journal's enabled writer is not the Session deletion writer.

No production signed bootstrap package was published and no portable floor was
initialized; native package fixtures used temporary signing keys and isolated
state. This release ships the installer admission code. Signed floor-bearing
bootstrap/recovery selection, publisher-key rotation, committed portable deletion
state and production deletion writing remain closed. This task activated no
Controller, Web, host configuration, installed Plugin or iOS component.

The [machine-readable evidence](../experiments/plugin-signed-bootstrap-bundle-2026-10-04.json)
contains exact source/build identities, owner and receipt, retained companion
hashes, complete process/HTTPS samples and validation boundaries.
