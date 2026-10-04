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
Machine protocol or accepted worker interface. Native acceptance and the actual
production component receipt are recorded after building the committed release.
