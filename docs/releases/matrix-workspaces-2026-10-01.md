# Hierarchical and mapped workspaces

New Session now selects Machine before Provider. The directory picker groups
slash-separated display names by default, with persistent opt-out, full-path
search, breadcrumbs, keyboard navigation and explicit selection of a directory
that also contains child projects. Folder navigation never selects a workspace.
Opaque IDs remain unchanged; the Machine's version-one workspace file supports
an optional `display_names` map. Unknown IDs and invalid labels fail closed.

OVH now advertises its original Matrix, Columbus and Suger roots plus 16 mapped
Hawk/Falcon roots. The Controller's actual Machine snapshot was checked for all
16 display names after a successful `cowboy operator refresh-machine --machine
ovh`. That delegated command uses the existing private authenticated host
boundary; it introduces no public unauthenticated refresh capability. Atomic
configuration replacement plus this command reloads the inventory without a
Machine restart. File replacement alone is not a live watcher.

## Remote task behavior

Matrix commit `b7c98bea` provides committed logical entry directories and shared
AGENTS/CLAUDE guidance. `./mx info` prepares or reuses an isolated target Git
worktree from a freshly fetched default branch. `./mx run` batches native shell
commands near source; `read` returns hashes and `write` rejects stale content
before an atomic single-file replacement. No mutable Git trees are mirrored.
Provisioned OpenSSH aliases, five-minute control connections and private socket
paths keep transport out of task prompts. Falcon currently follows an existing
authorized SSH chain; no keys or network policies were changed.
Infrastructure mappings resolve from the execution account's home even when
a Provider substitutes a private `HOME`; Provider credentials remain untouched.

Columbus owns the 16 route declarations, source selection and native workspace
manifest. Recent Hawk session metadata selected the common projects. Falcon's
selection uses actual checkout/task-worktree activity, not an invented session
ranking. Its generated Suger root had no Git metadata, so a separate clone of
the authoritative Suger repository now supplies isolated root tasks; existing
generated files and project checkouts were preserved.

This adapter does not redirect native local Read/Edit tools or Cowboy Code.
They see the entry checkout. Shared guidance directs remote work through `mx`;
full transparent remote editor integration is not claimed. Real-model token
usage and physical iPhone interactions were not measured.

## Verification and measurements

- Complete `just check-compact` passed for the workspace feature, including
  1,686 main Rust tests and 1,971 Web tests. Subsequent private-operator changes
  passed 27 boundary tests and the declarative workspace reload regression.
- Actual React/MUI in isolated Firefox verified default grouping, navigation
  without selection, selectable parent roots, full-path flat selection and
  saved preference. Final type checks and lint passed with existing warnings.
- Matrix's gate and five regressions passed, including private Provider homes,
  fresh default branches,
  dirty source preservation, failed fetches, task reuse, inherited source-side
  policy, stale writes, modes and path boundaries. Native review found the
  inherited-policy omission; it was corrected before final remote acceptance.
- Actual OVH calls through both interfaces verified Unicode/quoted contents,
  readback, stale-write refusal, remote assertions and exit status 37. All
  16 configured sources were confirmed as Git roots through their interfaces.
- Two normal Matrix Git worktrees, without `MATRIX_TASK`, independently created
  target tasks; repeated calls reused each task and separate tasks stayed apart.
- Five sequential samples per case: OVH-to-Hawk `ssh true` cold median 5.861 s,
  multiplexed median 1.480 s, approximately 75% less latency. Warm `mx run true`
  medians were 1.291 s for Hawk and 2.082 s for Falcon. Initial task preparation
  took 11.875 s and 7.164 s respectively and includes Git fetch/worktree work.
  These are one-time network observations, not a throughput or token benchmark.

The approach follows [OpenSSH connection multiplexing](https://man.openbsd.org/ssh_config#ControlMaster)
and the tool-near-source model of [Remote SSH](https://code.visualstudio.com/docs/remote/ssh).
[SSHFS](https://github.com/libfuse/sshfs/blob/master/sshfs.rst) and
[synchronization](https://mutagen.io/documentation/synchronization/) were considered;
filesystem caching and conflict reconciliation add unnecessary uncertainty for
this connected, single-authoritative-checkout workflow.

## Activation

[Exact receipts and samples](matrix-workspaces-2026-10-01.json) record:

- Web `730813dd`, service worker `cowboy-v1785`: accepted without restarting a
  process; origin HTML served with `no-store` and the new version hash.
- Controller `23feef74`: accepted through the machine-owned component activator;
  this includes the new delegated refresh command and the intervening published
  usage-repair changes. Health and version endpoints passed.
- OVH host `730813dd`: accepted through a separate bounded root-systemd
  transaction with an independent four-minute rollback timer. All eight retained
  ACP workers kept their PIDs, start times, executables and original generation.
  The Machine reconnected, Plugin identities stayed unchanged, normal process
  containment was restored, and the rollback timer was stopped after acceptance.
- Only then the full display-name manifest was installed and refreshed. Public
  deployment health reports the expected revision and 19-root inventory digest.

The previous compatible manifest remains available for an explicit old-host
rollback. Cowboy, Matrix and Columbus source changes are published on their
respective main branches. A PWA hard reload is needed to load the new picker;
WebSocket reconnection alone does not replace cached JavaScript.
