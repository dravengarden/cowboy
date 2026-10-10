# Portable Machine reader floor — October 4

Cached-host authentication alone did not prevent the portable launcher from
selecting a correctly signed undeclared predecessor, or falling back to an
installer-selected bootstrap after active links disappeared. An empty deletion
namespace carried no persistent minimum-reader intent across these changes.

A successfully verified singleton Machine host declaring schema-1 reading and
no writer now retains `portable-session-deletion-reader-floor.json` in its
canonical state namespace before active/rollback/command publication. The closed
schema-1 record binds that namespace, the fixed `session-deletions` dataset,
reader schema, normalized publisher key SHA-256, and the first anchor's version,
digest, generation and canonical signed-proof SHA-256. It authenticates the
anchor's original artifact, publisher signature and raw/archive contents.
The signed-proof hash pins entrypoint/probe/automatic/reader fields as well as
version/generation/digest; unsigned download URLs are not anchor identity.

Floor writes use a mode-0600 exclusive random staging file, file sync, exclusive
hard-link publication, and floor/state-directory/parent sync before selecting
the candidate. The first floor is retained unchanged. Unknown, duplicate,
missing, oversized, foreign-state, unsafe-path, symlink, nonregular, shared-mode
or foreign-owner records refuse without repair. Reads use no-follow/nonblocking
opens, a private effective-UID-owned regular-file check and an 8-KiB bound.
The writer never overwrites or clears a committed floor. A publication or sync
failure may leave that floor fencing an older active selection; no irreversible
floor commit is rolled back to restore an unadmitted bootstrap.

Reconciliation checks existing floor/anchor and candidate compatibility before
fetch, again after authentication and around the signed probe, then retains and
authenticates the floor before publishing pointers. A probe that removes or
changes a previously observed floor cannot publish or silently recreate it.
Changing an existing anchor's signed proof in the same version/digest directory
also refuses before fetching or writing. The accepted anchor stays protected
from cache pruning across later compatible updates and rollback-link movement.
Pruning refuses unreadable floor intent before deleting any generations.

Updated portable launchers require the floor's publisher and authenticated
anchor, as well as a signed declared schema-1 selected host, even over an empty
namespace. Missing active/command selection can no longer fall back to bootstrap.
Healthy compatible updates retain the original floor bytes. Absent floor and
absent active selection retain ordinary trusted-bootstrap behavior; legacy
signed hosts over empty namespaces do not create floors automatically.

Register/install/refresh refuse every floor entry before bootstrap, identity,
origin or launcher changes. The captured bootstrap must report
`host_cache_guard: 2` and demonstrate read-only refusal of both a committed
journal and a separate synthetic floor, with the existing deadlines and parsed
output bounds. A cache-only guard or a guard claiming v2 without floor behavior
is rejected before installation. Signed bootstrap and recovery admission are
still pending; this deliberate refusal keeps that boundary closed.

## Validation boundary

Raw/archive lifecycle fixtures create a floor from a signed declared host,
advance compatible versions, force pruning, and retain the exact first floor
and anchor. Signed undeclared downgrade, immutable-anchor proof replacement,
bootstrap fallback, corrupt anchor and signed-probe floor removal refuse.
A forced active-pointer publication failure leaves the committed floor and
verified anchor without publishing a command. Storage fixtures exercise closed
fields, duplicate keys, ownership binding, bounded reads, private modes and
special entries without replacing invalid state. Installer and bootstrap probes
cover pre-effect floor refusal and truthful capability behavior.

The exact-release generated-launcher fixture now includes twenty-four isolated
raw/archive cases, including healthy floored readers, absent active selection,
signed legacy selection, corrupt floor and independently corrupt retained
anchor beneath a newer healthy active host. Ordinary-start markers detect
unintended cached execution or bootstrap fallback. The preceding independent
guard's acceptance of absent selection despite the floor is a negative control;
the updated installer refuses that guard. Independent older administrative
tools and same-user state replacement remain outside this finite boundary.

This floor guards updated portable launchers and component reconciliation;
direct caller-owned native launches and the Nix owner's separate root reader
floor are distinct. It binds storage scope, not a Machine/Service security-domain
incarnation. It does not authenticate the caller-selected bootstrap, authorize
publisher-key rotation, supply signed recovery, fence a concurrent administrator,
undo arbitrary trusted probe effects, or prove power-loss/device/native-resume
acceptance. Retained anchor integrity is checked; anchor availability across
power loss and independently supplied writer-release acceptance remain separate.
Committed portable deletion state still refuses before selecting any reader,
and the production deletion writer stays disabled. Existing remote portable
installations need the new bootstrap/launcher and a verified declared component
before this floor can be established; Hawk activation does not refresh them.

## Accepted source and immutable release

Source `a2a5360ec58714c206213dc2d449ff0843b45c15` passed formatting,
all-feature and default-feature Clippy with warnings denied, 1,799 all-feature
library tests (44 ignored), integration tests, 491 standalone Machine tests
(7 ignored), Machine/Code feature checks and Plugin/Provider checks. The
immutable default Cowboy package, Machine host output, source-boundary check
and worker-registry input check built with their existing warnings and test
gates. No signature, namespace, floor or production writer gate was relaxed.

The actual accepted native matrix used
`/nix/store/xcrfzrwdp6k04lccs6fmjp6hgbc5ywhz-cowboy-machine-release` against
the preceding cache-only bootstrap release
`/nix/store/9rqny80agvhq8s5ni7ddacj8bps1m18b-cowboy-machine-release` and
pre-guard legacy installer release
`/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release`.
All twenty-four launcher cases and new/old bootstrap controls passed. The exact
new installer passed both legacy layouts, both captured mutable-bundle cases
and both independent old-installer negative controls. The old guard's two
absent-selection floor acceptances were observed explicitly, not described as
fenced authorities.

Remote main advanced with an independent native-shell Scene-lifecycle change.
It was integrated at `ff6e51264540e61122c317bbd786782b0c106523`; merged
Plugin and native-shell source checks passed. This merge changed no Machine
Rust sources or retained companions. The rebuilt final host artifact's native
path/digest and installer entrypoint digest equal the exact acceptance artifact;
only immutable metadata changed. Final source is published on remote main.
No native device build or release is claimed by this task.

The activated output of `.#cowboy-machine-host-release` is
`/nix/store/nhdfwgfsz63ji462cgp264v51ihsmp7g-cowboy-machine-release`. Its
native executable is
`/nix/store/qppsrwahzh9b34ibby863gf7qyw7hp65-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`,
SHA-256 `5bd07c701799d5e3f647fcf006ec3fc5ab5c12cf67cb28a997c49ffe0b57e406`.
The installer entrypoint SHA-256 is
`00a57c927bb751358b36e05463238cd77996e8aabf8db11fce9153524460fb94`.
All six worker/proxy/Code/Zed/JS companion paths and digests equal the preceding
active bundle. Their separately retained source remains
`406471a28de430debf6f8363b44abc3e621589d7`, generation
`worker-6ede7a91cc8b8b3402d4`.

## Production receipt

Installed-owner transaction `1791093948961277128-ff6e51264540` succeeded,
published and committed at `2026-10-04T06:05:59.624767715Z`, with
`maintenance: true` and no recovery. Its predecessor is the exact preceding
`9rqny80agvhq8s5ni7ddacj8bps1m18b` release. The independently installed owner
and host configuration stayed unchanged; its root systemd unit finished
successfully. Both component in-progress journals are absent and no failed
system unit was observed or reset.

Samples at `2026-10-04T06:05:34.810Z` and `2026-10-04T06:06:33.631Z`
preserve all thirteen worker and five keeper unit IDs, states and PIDs exactly.
Only the resident Machine changed, from PID `2088278` to `3171975`.
`/proc/3171975/exe` matches the accepted native path and SHA-256. Controller
PID `959309` and receipt `1791079277476212104-406471a28de4` stayed unchanged.
This is bounded Linux process continuity, not general native Session resume.

All five public HTTPS checks returned 200; the Machine reports connected and
online on the retained worker generation. HTML/SW kept `no-store`; Web profile
and SPA version stayed unchanged. The independent root reader-floor SHA-256
`26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017`
and sudoers SHA-256
`149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`
were preserved. At `2026-10-04T06:05:49.039569Z` the Machine logged zero deleted
IDs and `writer_enabled=false`; deletion entries remain only `.lock`.

No production portable floor was initialized and no new signed production
component record was published. The persistent-floor implementation ships in
the Machine/installer code; an actual portable installation establishes it only
after authenticating its first declared signed host. Signed bootstrap/recovery,
publisher-key rotation, committed portable state and production deletion writing
remain unadmitted. No Controller, Web, host configuration, installed Plugin or
iOS activation was performed by this task.

The machine-readable evidence (in Git history)
records exact sources, builds, native/installer/retained identities, immutable
acceptance controls, owner, success transaction, and complete before/after process
and HTTPS samples.
