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

Exact source/native checks and production activation evidence are appended after
acceptance. The release uses `cowboy-machine-host-release` and retains the already
accepted `c4f29d40ec3854546ab2bbe54ff7237105f1d353` worker bundle; no pin advancement
or new worker/adapter generation is part of this slice.
