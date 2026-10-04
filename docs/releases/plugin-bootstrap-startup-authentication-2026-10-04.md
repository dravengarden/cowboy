# Portable bootstrap authentication before startup — October 4

Signed installation previously authenticated the package only during installation.
It discarded the manifest/archive and ran the installed bootstrap diagnostic
before any future byte authentication. Modifying that executable could therefore
run code before the launcher checked cached-host admission.

Signed installation now captures the original manifest and archive alongside the
three authenticated payloads. Before the publisher probe it also captures the
running installer executable as an independently owned verifier; its streamed
digest is checked after the probe and after copying. A fresh mode-0700 generation
under `signed-bootstrap` contains exactly these six regular files. Payloads and
verifier are executable; evidence files are mode 0600. Files and generation/state
directories are synced before the launcher is atomically published. Refresh
creates a new generation and does not overwrite a previously selected one.

The generated launcher calls the fixed generation's verifier first. It validates
the original closed reader-only v4 manifest, configured publisher signature,
whole-package digest, exact archive layout, exact installed payload bytes and
executable access. It independently checks portable namespace and selected cached
host/floor admission before executing any publisher code. Failed authentication
stops before the bundled diagnostic, companion detection or runtime-directory
creation. Only after verification does the existing bootstrap diagnostic run and
the launcher select the active host or trusted pre-floor bootstrap. The original
size bounds and canonical signature representation remain unchanged.

The verifier, launcher, configured key and host runtime libraries remain external
administrator-owned authority. The verifier does not authenticate itself before
its own execution. Signed probes are not sandboxes; concurrent same-user or
administrator mutation is outside this check's boundary. Independently authorized
older tools remain usable under that authority. No stronger isolation is claimed.
Earlier flat installs require an explicit pre-floor signed refresh; no automatic
migration occurs. Partial failed staging retains unselected directories, and old
generations have no automatic garbage collection. Atomic launcher publication
does not make the whole installer transactional or prove full power-loss recovery.

Floor-bearing register/install/refresh still refuses offline before Controller
discovery. This change creates neither a portable floor nor a cache recovery
anchor, does not permit bootstrap fallback after floor/cache loss, authorize key
rotation, admit committed deletion records or enable the deletion writer.

## Separate accepted worker pin alignment

The prior host-release input pointed at source
`406471a28de430debf6f8363b44abc3e621589d7`, generation
`worker-6ede7a91cc8b8b3402d4`. Production subsequently accepted full Machine
source `c4f29d40ec3854546ab2bbe54ff7237105f1d353`, generation
`worker-9fce17441fdd1e8ca642`, in transaction
`1791098241083314286-c4f29d40ec38`. Its independent transport/native worker
acceptance is recorded in
[`execution-transport-deployment-2026-10-04.json`](../experiments/execution-transport-deployment-2026-10-04.json).

This slice separately aligns `cowboy-workers` with that exact already accepted
production source, NAR hash
`sha256-80o36/+ih4jV72ALxeKEcEHw7Ya8WsE/oqZ7tdlADTk=`. The root Nixpkgs and
Rust overlay revisions remain unchanged. The release must retain production's
six companion executable paths and SHA-256 digests and the same default worker
generation; activation is conditional on that comparison. Native protocol, wire,
SDK and dependency source guards are retained without bypass. Build output is
`cowboy-machine-host-release`; no new pool or adapter generation is accepted here.

## Validation and production receipt

Source gates at `ec52587ad5a54a2123b47986db2b305d4b6098c7` passed all-feature
library tests (1,809 passed, 46 ignored), integrations, standalone Machine tests
(498 passed, 9 ignored), all-feature and default Clippy with warnings denied,
Rust formatting and Plugin/Provider checks. The final merge incorporated only
independent Web changes and its production receipt. Final immutable source-boundary
and worker-registry checks passed; the default Nix package passed 1,330 tests,
with 27 ignored, without retry. Its Rust package derivation was unchanged by that
Web merge. No signature, warning, assertion or source guard was disabled.

The exact final release passed four native tests: nine signed startup cases
(healthy; mutated host, Code, worker, manifest or archive; missing or linked
archive evidence; invalid floor), signed native installation and offline floored
refresh refusal, all twenty-four raw/archive cached-launcher cases, and current
versus preceding cache-only bootstrap guard admission. In the signed startup
matrix a marker at the first instruction of publisher code appears only for the
healthy case. Every refusal leaves the marker and runtime directory absent.
The matrix uses disposable signed fixtures and does not claim provider inference
or a production signed bootstrap publication.

Published source `ae9d08c8881c56e1f742a75826cbe7ecc735b3dd` was activated from
`/nix/store/6af4mb660ry4crpb4gazshhn5fdm2d9q-cowboy-machine-release` by the
unchanged installed owner. Transaction `1791100992954474633-ae9d08c8881c`
succeeded, published and committed at `2026-10-04T08:03:23.586767353Z`, with
maintenance enabled and no recovery. Its predecessor is the already accepted
`vhwiwp321mr44hry5csrr9c9lkd4yb07` release. The actual running native is
`/nix/store/f8w80j9l2r3bbii3wdjnylhip8cph6hv-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`,
SHA-256 `92b3458f7cab1e2749c1c7ad89f27e7d0af6ba1b40218b577a8c64aa9cf7cf40`.
The installer entrypoint SHA-256 is
`f31a06c20068b375e8d3eb812b6e1b9c4f7423430c03ad440a8c6a5ab684b3ef`.

Both the immutable comparison and production receipt retain the same six
companion paths/digests and `worker-9fce17441fdd1e8ca642`. The separately aligned
pin is recorded as the accepted full Machine source in
`retained-worker-source.json`. This is retention of that existing generation,
not a new worker rollout.

Before/after samples retain all thirteen workers and five execution keepers with
the same IDs, states and PIDs. Resident Machine PID changed from `4090042` to
`303991`; its `/proc` executable and digest match the artifact. Controller PID
`959309` and receipt, Web profile/version, root reader-floor bytes and sudoers
bytes remain unchanged. `/healthz`, `/version`, SPA, service worker and Machine
deployment health returned 200; HTML/SW retain `no-store`, and Machine reports
connected/online with the retained generation. Both component in-progress files
are absent; no failed system/user unit was observed or reset. These are bounded
process samples, not proof of a new native-generation swap or session resume.

The deletion namespace remains only `.lock`, no portable floor is initialized,
and the actual new Machine startup reports zero deleted sessions with
`writer_enabled=false`. The independent Plugin-incarnation writer's true flag
does not describe the deletion writer. Sudo remains available and its policy
SHA-256 remains
`149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`.
No Controller/Web restart, Plugin operation or production signed bootstrap
publication was performed by this slice.

Exact executable digests, process timestamps and activation receipts are in the
[machine-readable evidence](../experiments/plugin-bootstrap-startup-authentication-2026-10-04.json).
