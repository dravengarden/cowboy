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

Source and immutable release acceptance, the exact artifact comparison and the
production activation receipt are appended after their checks complete.
