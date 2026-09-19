# Zed Budget rollout — 2026-09-19

Status: the separately authorized **Hawk Machine maintenance and signed Zed
`1.18.0` publication/installation are complete**. The installed release is bound
to the accepted exact native pair. Controller/Web stay on their previously
activated readers. This is not whole-refactor completion, adoption of retained
native owners, supported-device acceptance or independent post-effect recovery.
The old NixOS cold Controller has a separately recorded, pre-existing Catalog
compatibility failure; this record does not claim that reader passed.

This continues the [Budget reader acceptance](sync-budget-outcomes-2026-09-19.md).
Build/sign/maintenance source is clean, published
`94382b94e8275c6ad21cfdb9da1907bf1dab8d7a`, with Zed subtree
`c52e43d76d11c9fbb22079d9b2cb51e3b9e6dd2b`. Its production implementation is
unchanged from that acceptance: `17444b80` supplies the Nix fixture correction,
and `94382b94` adds acceptance documentation. The later remote Claude repair
`3f82f19c` was integrated before writing this record; it was not built,
published or activated by this rollout.

## Exact release and authority

Actual installed Zed Plugin/private adapter advances **1.8.0 → 1.18.0**, and
its selected private server **1.0.0 → 1.4.0**. The latter is the already verified
server candidate, not a new upstream update. Upstream remains
`aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`; no third-party dependency pins
change. Plugin component release `3.11.0`, SDK `1.8.1`, Code component `1.2.0`,
Code payload schema 2, outer Zed release schema 1, native/adapter API 1 and
Machine protocol 21 remain as accepted. Ordinary Zed is untouched.

| Artifact | Exact immutable identity |
| --- | --- |
| Machine release | `/nix/store/5f6x6dw6j2kgl30c4jdbz7b8pf2sbfz7-cowboy-machine-release` |
| Machine wrapped ELF SHA-256 | `31075c9d3f9e26804bb6d3833b8f03eea3b428e9f10dc8a81bd0185d3c8d0f90` |
| Adapter output | `/nix/store/6q1xw6phni7jjjlm6dsjwrfq5b7r2y3d-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.18.0` |
| Adapter ELF SHA-256 | `a2bf32aaf46277620ee700fff3f874dfe295de92eec422bec53887166ba10ace` |
| Private server output | `/nix/store/88hccb0s3csayvay7qwisn755pbscp4y-cowboy-zed-server-x86_64-unknown-linux-musl-1.4.0` |
| Private server ELF SHA-256 | `1bfd5b9556f61545906a0f24b08bdfcb96c194a900fc54b8d96992a012dec283` |
| Package SHA-256 | `c4fd0bdd96f326c5c10e154d96c0ea21968fb92e00d7faf0b30bf8a5d71b0d2f` |
| Composite artifact SHA-256 | `92b1078ab8bd8030cda30f32e13c7c7b355ffbd4e9a9dda0a8085c23c0ef89a1` |
| Contract fingerprint SHA-256 | `549778b1f7bba61fefcadc757c16a0f4083bb77a2d58475444c33b66259167d0` |

The repository-owned release skill, package-owned runtime builder and generic
SDK built/bound/signed the release on clean `94382b94`. The independent immutable
SDK verifier is
`/nix/store/bdjawvlvkv9w0hfbq8aljwb9rsk5da8c-cowboy-plugin-pack-1.8.1/bin/cowboy-plugin-pack`.
The existing `cowboy-first-party` publisher signature verifies against the
independently configured trusted public key, fingerprint
`SHA256:a/VJzmHD/94vMVQMNZTktSR9P3apkkKnnWzXrtn02hg`.
No new publisher, key rotation, credential copying or login was needed.

## Verification: reused versus repeated

The complete `just check-compact` acceptance on `f758f9cf`, final Nix
source-boundary gate on `17444b80`, exact static pair, temporary signed lifecycle
and Firefox suites are [retained evidence](sync-budget-outcomes-2026-09-19.md#gates),
not new full source/native/browser runs. In particular, the earlier 43-test
native build is not relabelled as a new run. No production source changed for
this publication.

The following gates ran again in the pinned shell for this rollout:

- Clean immutable Machine and SDK builds, official Plugin/runtime build,
  production binding/signing and independent signature verification.
- **72 Machine installation-reader checks, 7.64 s**, against the new release,
  pre-maintenance active/recovery Machine and actual NixOS cold Machine. This
  reader acceptance does not authorize a native-generation restoration.
- **19 connected v6 checks, 170.74 s**, using the actual active Controller,
  exact new Machine wrapper and exact static native pair. Disposable product
  password authentication, Machine enrollment and signed HTTP installation run
  through the four-process chain. Four lost actual replies retain the normal
  40-second deadlines, including original-ID Budget reconciliation. The receipt
  records `stage=complete`, `failure=null`, `cleanup=true`, `accepted=true`.
- Actual Machine-owner telemetry writer preflight: the required schema is
  present and the writer policy remains `unconfigured`. It does not inspect
  private destinations or authorize export.
- Full **86-entry** staged and published Catalog reads: each phase runs two
  independent process reads for each of the two Controller roles below, plus
  their two actual-Service host preflights. Host telemetry writer/background
  policies remain unconfigured; legacy selection remains `not_checked`.
- The three public HTTPS artifact downloads match the package/adapter/server
  digests above, immutable cache headers and digest ETags. Public unauthenticated
  `/api/plugins` remains HTTP 401; the authorized Operator Catalog supplies the
  installable release observation. The [website Plugin list](https://dravengarden.github.io/cowboy/plugins.json)
  separately advertises Zed `1.18.0`; website copy is not installation authority.

## Controller reader boundary and cold failure

The accepted Controller roles are:

- Actual active **and next-transaction recovery**:
  `/nix/store/ygyk7dd8r0c92fxmk6a2zb47rrw52ndh-cowboy-controller-release`.
  The component activator captures the active profile for transaction recovery.
- Retained predecessor:
  `/nix/store/16c48qyzmj8c2r8g864wyqzlgmpnif65-cowboy-controller-release`.

Their staged/published receipts explicitly say
`scope=active_and_actual_recovery_plus_retained_predecessor_only`.
They are not a three-role active/recovery/cold acceptance.

The actual `/run/current-system/activate` cold Controller pin is
`/nix/store/cc09k6l788mhchy321ckgg0yryb1hg12-cowboy-controller-release`, source
`869c269fc86e274f7c62e7faf69fd53a1b82893f`. It fails both the **original
85-entry live Catalog without this release** and the staged Catalog with Zed
1.18, on the same existing `claude-code-3.1.27` package. That package SHA-256 is
`c85cc31808885fd7f6b23b14fd45faad6aec76f96146c2f9bd2bc25d44ab8349`;
the error is an unsupported untagged `RuntimeValue`, not the new Code package.
The retained negative receipt has `accepted=false` for both observations.
This proves a pre-existing cold-reader gap, not successful cold acceptance.

No Catalog history was removed, decoder relaxed, Agent downgraded or cold pin
silently changed. Repair needs a separately owned, freshly fetched Columbus
worktree, compatible complete host/recovery configuration and its own
maintenance acceptance. The unchanged host closure is
`/nix/store/ik5hk6w9lrsbl339pvskvkvyihp9s38p-nixos-system-hawk-26.05.20260731.5b4f72e`.
The passing cold **Machine** installation reader is distinct from this failed
cold **Controller** Catalog reader.

## Production transactions

The installed machine-owned activator performed one explicit `--maintenance`
transaction, `1789781297580200413-94382b94e827`, committed at
`2026-09-19T01:28:25.846407404Z`: published, successful, committed,
`maintenance=true`, `recovered=false`. Its current receipt is under
`/var/lib/hawk-component-deployments/cowboy-machine/current.json`.
The new resident Machine is online on **`worker-3c1c8899619631af0662`**, and its
actual wrapped ELF matches the connected receipt. Previous/recovery Machine:
`/nix/store/lmry3df0ncmf1fb8pmjp75v8p47yzizr-cowboy-machine-release`, generation
`worker-89a1025ff0eb11271ede`.

Canonical `just plugin-publish` completed at `2026-09-19T01:29:21.303Z` in
`/var/lib/cowboy/plugin-catalog`. Its immutable receipt is
`receipts/zed-1.18.0-92b1078ab8bd8030cda30f32e13c7c7b355ffbd4e9a9dda0a8085c23c0ef89a1.json`.
Only Zed was published by this task. After the published Catalog reader checks
and refresh, the existing delegated host Operator resolved the exact release
and performed one normal upgrade:

- Operation ID: `hawk-zed-1-18-0-budget-92b1078ab8bd`.
- HTTP **204**, Service phase **`completed`**, Machine receipt **`applied`**,
  `requires_reconciliation=false`.
- Actual inventory: active Zed `1.18.0`, exact composite/contract digests above,
  installation revision
  `installation-6a9ccaf6228fc41d942a05b9520a0b24868277660825e284e02a350b59d3f9b4`.
- Retained rollback generation: Zed `1.8.0`,
  `sha256:56474a7197fb8ba30d401236e780a35f107a9e9e7a5ab9869445c0d53a425d20`.

No repeat upgrade, direct database edit, pointer write or worker restart was
used to manufacture an installation receipt. Keeping a rollback generation is
not execution or acceptance of post-effect restoration.

## Continuity and exclusions

Immediate and settled audits through `2026-09-19T01:33:36.974Z` retain all **16
original ACP worker PID/start/executable identities** and their units. The
resident Machine changes as authorized. One additional ACP worker appears in
the settled snapshot; no original worker disappears. There was no original
private Zed process in the baseline, so this is not a native-owner resume test.
The observation before installation still reports 16 draining workers and two
busy workers, with handoffs/pending zero. **Drain completion is not accepted.**

Controller PID **1051040**, its release/receipt, Web release/receipt, host
closure, the three Victoria PID/start observations and all seven other Plugin
installation identities remain unchanged. The Web output remains
`/nix/store/4zzxb0jfgnv346ljhx691mjl8k15qhli-cowboy-web-release`;
`/version` remains `4bb51ded1747cbc6ffef8a7940736881` (SPA hash, not Git revision).
Local/public health succeeds. Each audit passes six local/public HTML, SW and
entry-JS byte/ETag/cache checks: HTML/SW `no-store`, hashed JS immutable.
PWA clients still need the explicit Update/reload for the preceding Web release.

The production private navigation policy remains closed. This release does not
perform a production native Budget mutation, migrate historical unknown owners,
complete a native resume, validate a supported physical device or change managed
Victoria admission/destination policy. General graph/site/state leases,
all-writer/global native history/snapshot/background limits, other capability
acceptance and independent post-effect recovery remain in the
[completion ledger](../plugin-refactor-completion.md).

## Retained evidence

Private evidence: `/tmp/cowboy-zed-rollout.xOqdvi`, with the build/sign logs,
staged public Catalog, bounded host observations, one installation intent and
separate actual inventory/operation captures. No private key or Service
credentials are included in this document.

| Evidence file | SHA-256 |
| --- | --- |
| Final runtime build receipt | `343f2b14c74aae676dfdb23683cee5260bb7275522b9d630f64640dce1bc1cdd` |
| Signed release envelope | `d722dfd678f1135111c1fc92dc9da637c87110b74c93670b5ed5102ca2e63316` |
| `machine-readers-receipt.json` | `62203beb28d0676d063c27a2906cc8f32e7d8ea513cacfa00ff932062c804c34` |
| `connected.json` | `51eea160bbcb06324878e49814fa6fca2bc7a03446e5c00f602a3dc36fea84ee` |
| `staged-actual-recovery-readers.json` | `622d583c9fa9f62d2fdd31d957f0ccd3e2f706e861d0371e128103254d5b40ac` |
| `published-actual-recovery-readers.json` | `622d583c9fa9f62d2fdd31d957f0ccd3e2f706e861d0371e128103254d5b40ac` |
| `cold-controller-negative.json` | `2e10bb26bb8dcecaaa0581b67760388417d6d313893ced0144bc4a6d96ef31a6` |
| `public-downloads.json` | `9e9005df7844ea2f72969f75ca7ac5ddd3cf0ccff27da7e9d3ff2bf83caa32f8` |
| `audit-after.json` | `3f16daf274ae185cca01f0b7b7653353879b1d6423125165eec8c8645c311f01` |
| `audit-settled.json` | `97ec97320e840bc135418e08f37c5061358cb07ea91ab5289479086ec0c1e96f` |

Setup diagnostics remain failures, not product gate passes: an initial public
key byte comparison differed only by a newline; isolated preflight needed
parent-owned `/proc` reads and the pinned verifier PATH; a private guard typo
failed before any maintenance dispatch. Corrected runs preserved product guards.
The cold Controller failure above remains unresolved, not a setup correction.
