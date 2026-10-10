# Staged Machine host integrity — October 4

Component reconciliation verified the freshly downloaded artifact digest and
publisher signature, but reused an existing cached executable without comparing
its bytes. An altered cached host could therefore run its health probe and be
published under the authenticated artifact's manifest. A correctly signed probe
could also modify the staged payload before publication.

Machine host reconciliation now prepares expectations from the authenticated
download, checks staging before writing its manifest or running its probe, and
checks it again before changing active, rollback or command pointers. Raw hosts
must be regular files with the exact signed bytes. Archive hosts must have the
exact regular-file/directory tree and file digests, including companions and
empty directories. Expected archive paths are normalized before extraction;
duplicate files, file/directory conflicts and special entries refuse. Staged
links and extra entries refuse. File reads use no-follow/nonblocking opens and
fixed-size hashing buffers. Other component kinds keep their existing behavior.

An altered cache is refused without repair or probe execution. A signed probe's
effects remain on refusal; no rollback of arbitrary probe effects is claimed.
The two checks observe staging at separate points; they do not create an
immutable filesystem snapshot or fence a concurrent administrator. This does not authenticate an
already cached host at a later launcher start, supply signed bootstrap recovery,
create a portable persistent floor or admit committed portable deletion state.
The production deletion writer remains disabled, and the user's retained sudo
authority remains unchanged.

Source acceptance covers changed raw bytes, a raw symlink, changed archive
companions, extra archive files and archive symlinks. These candidates refuse
without executing a marker probe, rewriting a retained manifest or changing
retained pointers. Raw and archive probes that alter their own staging run once
but cannot publish. Unchanged raw/archive hosts activate both on initial staging
and cache reuse. Additional archive fixtures reject duplicate and normalized
duplicate files, both file/directory conflict orders and FIFO entries. All keys,
payloads, markers and state in those tests are synthetic temporary fixtures.

Implementation commit `5afd057e` was integrated with fresh main as
`4f70a607f6033f47afd0c48bb57f109454f92baa`. The incoming Sessions-folder
changes are Web-only; their lint and type checks passed after integration, and
Web tests/build also run against the merged source. This task activates only
Machine. Final activation evidence follows after the immutable release. No
production portable install or signed component record is published by this slice.

The first complete gate refused the previously integrated native app-shell
change `eebf0a5c`: its source digest had not been recorded in the component
registry. Append-only release 3.38.0 records app-shell 1.1.19 and its new digest;
there are no dependent component/Plugin changes. Historical registry entries
and all Plugin pins remain unchanged. This repairs the deterministic gate rather
than weakening it. This task does not activate Web or native app releases.

The complete gate's static checks, Clippy with warnings denied, dependency audit,
type/feature checks and all-feature suite passed (1,784 passed, 42 ignored).
Its parallel independent Machine suite hit two existing failures:
`committed_terminal_ids_survive_close_and_read_only_reopen` reported a busy
journal after drop, and `empty_success_alone_cannot_admit_a_bootstrap` did not
reach its expected refusal assertion. Neither was suppressed. The entire
Machine suite was rerun serially: 476 passed, five ignored; both failures did
not recur. This is consistent with transient concurrent subprocess interaction,
not proof of its cause or a fix to those tests. Code-adapter and Zed suites,
merged Web tests, isolated PostgreSQL fixtures
and release builds all passed in the resumed gate. The gate was completed in
these stages; the initially failing `just check-compact` invocation itself is
not reported as a successful run. No production guards or tests were relaxed.

The initial Nix package from `0fbaf9e5` failed because the isolated Machine
source list omitted the new Rust submodule directory. Commit `fc40fafe` adds
that directory; its Machine release and source-boundary check built. This
candidate was not activated: verification found that the app-shell registry
repair changed the conservative whole-registry worker hash to
`worker-cf06753db70e910f6c60`. The existing release wrapper and Machine hello
would select that generation, unnecessarily rolling detached workers. Neither
candidate dispatched a component transaction or changed the production receipt.

The final change derives a worker-relevant registry prefix from the current
append-only registry. Only consecutive app-shell-only records with no internal
component or Plugin shell consumer can be excluded. Other component identities,
Plugin versions/source/pins and dependency changes retain the latest input.
Before the subsequent Claude merge, the derived descriptor records release
3.37.0 and its genuine historical JSON
SHA-256 `5c6f56d976ffe38e51b578f22facee92f4bd2bcc7b9827fd8f8152db270c1e72`.
The root component gate and immutable worker build independently recompute it;
stale checksums, manually pinned releases and extra fields refuse without writes.
No desired-generation override or startup/rollout authority change is made.
That source's Nix evaluation retained `worker-6ede7a91cc8b8b3402d4`.

The five new TypeScript tests cover exact historical byte preservation across one and
multiple shell-only releases, unchanged derived input, changed SDK/Plugin source,
version, pin and graph inputs, shell consumers, invalid registry heads, the real
repository descriptor and actual CLI success/refusal in isolated temporary
directories with a cleared environment. The complete component gate passed with
this checker. These are generation-input checks, not native resume acceptance.

Fresh-main integration subsequently includes the independent Claude range-read
Plugin release and registry 3.39.0. Its changed Plugin source/pins legitimately
advance the derived registry input to 3.39.0 with SHA-256
`3f6457f1a1cfeb77888f4af4f88f653fc3297f15cf6ea4e1fdfa0c6b7a112d99`;
the shell-only exemption does not hide that change. Component and Provider gates
passed after integration. This task publishes or installs no Plugin artifact.

The new `.#cowboy-machine-host-release` is the narrower resident-host output.
It retains the separately accepted worker/Code/Zed/JS bundle from the exact
Cowboy `406471a28de430debf6f8363b44abc3e621589d7` flake input, pinned with its
Nar hash. Current Machine, installer and execution-host binaries are built
normally. The selected worker generation comes from that genuine retained
package; this does not relabel a new worker or read a mutable production profile
as a build input. Both source receipts are packaged. SDK, Cargo dependencies,
runtime/execution wire and Machine protocol bytes must match before this output
can evaluate. The current retained source passes; overriding it with pre-reader
claim source `abbbd77a` fails the actual Nix assertion without writing the lock
file or dispatching a transaction. Pool/adapter upgrades keep their independent
maintenance boundary. No general native compatibility or resume is claimed.

The first resident candidate from `e64ae98f` built and verified, but dispatch
refused stale ancestry after remote main advanced with documentation only. No
root transaction or new receipt was created. Fresh integration `1803851e`
rebuilt immutable metadata; native and retained companion paths/digests stayed
identical. The installed owner and its journal were retained throughout.

Final source `1803851ea511b0230570bac688dbfe2a5185ee7f` built `.#cowboy-machine-host-release`,
the source-boundary check and the immutable worker-registry input check. The
activated artifact is `/nix/store/z4pmjs2j2jy5093vljzzif72vmxbxicz-cowboy-machine-release`. Its native Machine executable is
`/nix/store/fi5sq3s9bxxi4ixvmlc5gzgzzwkmrapn-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`, SHA-256
`ac2c93835c2589e9cd716469999e6ae15e6432c86e5e70cff1b5061d01bf1028`. The separate retained source receipt pins
`406471a28de430debf6f8363b44abc3e621589d7`; all six worker/proxy/Code/Zed/JS
companion hashes equal the preceding active bundle. The actual worker remains
`/nix/store/s4dsd8zbcrn18995krfwciiszhz27nwn-cowboy-0.1.0/bin/cowboy-acp-worker`.

Installed-owner transaction `1791086185838758576-1803851ea511` succeeded and
published at `2026-10-04T03:56:33.839691692Z` with `maintenance: true` and no
recovery. Machine changed from PID `965129` to
`1442220`. Samples at `2026-10-04T03:55:56.643Z` and
`2026-10-04T03:58:18.403Z` preserve all 13 worker and five keeper unit
IDs, states and PIDs exactly. Controller PID `959309` and its
receipt remain unchanged. The selected pool stays `worker-6ede7a91cc8b8b3402d4`.
This is bounded Linux process continuity, not general native resume acceptance.

All five public HTTPS checks returned 200; HTML/SW retained `no-store`. The Web
profile and SPA version `798bda6db1a3a8958a6102125058e8e2`, root reader-floor
SHA-256 `26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017`, installed-owner hash and sudoers
SHA-256 `149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69` stayed unchanged. Both component
journals are absent and no failed system unit was observed or cleared. The
Machine logged zero deleted IDs with `writer_enabled=false` at
`2026-10-04T03:56:25.922499Z`; deletion state remains only `.lock`.

The [machine-readable evidence](../experiments/plugin-host-cache-integrity-2026-10-04.json)
contains artifact/native/retained identities, exact owner and transaction, initial
stale refusal, complete process/HTTP samples and the unchanged boundary hashes.
Portable cached startup authentication, persistent floor, bootstrap/recovery
admission and production deletion writing remain closed. No Controller, Web,
host-configuration, installed Plugin or iOS activation was performed by this task.
