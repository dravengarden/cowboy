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

Exact build/native acceptance and production receipts will be recorded after
completion.
