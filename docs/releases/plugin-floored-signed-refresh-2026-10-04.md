# Signed refresh with an existing portable reader floor — October 4

The preceding installer refused every floor-bearing refresh, even when an enrolled
Machine retained a valid signed anchor and selected compatible host. The startup
verifier could authenticate those records, but refreshing its bootstrap still
required removing the floor, which is not an accepted operation.

The Machine-host installer now admits signed refresh without removing or replacing
that floor. The existing namespace must remain uncommitted. The candidate closed
reader-only v4 three-program package must authenticate under the configured key;
the floor must bind that publisher and canonical state directory; its retained
anchor and both selected cache pointers must independently authenticate. Local
failure precedes Controller discovery, publisher probes or installation writes.
Normal enrolled Machine identity, origin, Service and token checks remain in force.

Admission captures the floor's exact bytes and the literal targets of both
selection symlinks. After the bounded compatibility probe and before publishing
the launcher it repeats candidate/cache authentication and compares those records.
A changed, removed or newly created floor or changed selection refuses publication.
No altered record is repaired or erased. Probe effects remain evidence rather
than being rolled back to manufacture admission.

Successful refresh retains the floor, anchor, current host selection, identity,
Machine id and token. It installs the independently verified bootstrap into a new
private generation and publishes the guarded launcher using the preceding startup
authentication design. Older generations remain intact. Startup independently
authenticates the new bootstrap and the existing selected cache before execution.

This is refresh of an already authenticated selection. New enrollment under a
floor, unsigned refresh, missing or incomplete selected cache, corrupt anchor or
floor, mismatched publisher, key rotation and committed deletion state remain
closed. It creates no floor, selects no recovery candidate and enables no deletion
writer. Installers built without the Machine-host feature still refuse floor
refresh. Independently authorized older tools and same-user/admin races remain
outside this boundary; the checks are not a filesystem lock or probe sandbox.
Launcher publication is atomic, but the whole installer is not a transaction or
full power-loss recovery mechanism.

## Verification and production receipt

The release uses `cowboy-machine-host-release` and retains the already
accepted `c4f29d40ec3854546ab2bbe54ff7237105f1d353` worker bundle; no pin advancement
or new worker/adapter generation is part of this slice.

Final source passed all-feature library tests (1,810 passed, 47 ignored),
integrations, standalone Machine tests (499 passed, 10 ignored), all-feature and
default Clippy with warnings denied, Rust formatting and Plugin/Provider checks.
Immutable source-boundary and derived worker-registry checks passed. The default
Nix package passed 1,330 tests, with 27 ignored, without retry or disabled checks.

The exact immutable release passed a five-case native refresh matrix. The
preceding release rejects a valid floor before contacting the disposable loopback
Controller. The current release refreshes that same enrolled fixture successfully,
keeps exact floor bytes, both cache pointers, identity, Machine id and token, and
starts the generated launcher through the existing selected host. The previous
private bootstrap generation remains byte-identical and a second generation is
created. Corrupt anchor, absent cache, invalid floor and tampered package all refuse
with zero Controller requests, one unchanged bootstrap generation and unchanged
launcher/identity/token/selection. Damaged evidence is retained without repair.
These disposable fixtures do not use production signing credentials or claim
real Provider inference or cross-host enrollment acceptance.

Native regression acceptance also passed all nine signed startup cases, signed
installation/offline malformed-floor refusal, twenty-four raw/archive cached
launcher cases and current-versus-preceding cache-only guard admission. Source
fixtures additionally refuse floor-byte and literal-pointer changes after
admission, committed state, wrong publisher and incomplete selection.

Published source `80e0b4acb9d5ae9f64f114a92b184d8ffc60abe4` was activated from
`/nix/store/hm3h8lgjgdvqyvi7i1bgr5wm9v91wjfl-cowboy-machine-release` through the
unchanged installed component owner. Transaction
`1791102172215759610-80e0b4acb9d5` succeeded, published and committed at
`2026-10-04T08:23:00.718434572Z`, with maintenance enabled and no recovery.
Its predecessor is the startup authentication release
`6af4mb660ry4crpb4gazshhn5fdm2d9q`. The actual running native executable is
`/nix/store/ss0n2vy8y2m3j4ivc694ifp8frvv0mkb-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`,
SHA-256 `f428eb5843e8ad25d7f77f32301391df75bd3f4dd868bad31c50ce0f498fdc6b`.
The installer entrypoint SHA-256 is
`59a82ea096bd61f2a970c7416ff1c40c83bdb2b968bba6ec76716f5c767f38eb`.

Samples at `2026-10-04T08:22:33.330Z` and `2026-10-04T08:23:29.326Z` retain
all thirteen workers and five keepers with the same IDs, states and PIDs. Resident
Machine PID changed from `303991` to `452169`; its `/proc` executable and digest
match the artifact. All six retained companion paths and digests, default
`worker-9fce17441fdd1e8ca642` generation, Controller PID `959309` and receipt,
Web profile/version and root reader-floor bytes remain unchanged. These are
bounded process samples, not a new worker-generation swap or session-resume test.

All five HTTPS health/version/SPA/SW/deployment-health observations returned 200;
HTML/SW retain `no-store`, and Machine is connected/online with the retained
generation. Both component in-progress files are absent and no failed system/user
unit was observed or reset. The deletion namespace remains only `.lock`, no
production portable floor was initialized, and the new native startup reports
zero deleted sessions with `writer_enabled=false`. No signed production bootstrap
package, Plugin operation, Controller/Web restart or missing-cache recovery was
performed by this slice.

Sudo remains available; its policy SHA-256 remains
`149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`.
The machine-readable receipt (in Git history)
retains exact sources, executable digests, process samples and component receipts.
