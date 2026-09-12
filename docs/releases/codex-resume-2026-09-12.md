# Codex resume transport — 2026-09-12

Source `7e59b22f5b024535fe64b98e2131d39bf1e94eb2` is integrated into remote
`main`. Codex 3.1.18 is signed and published as immutable Catalog files. Service
Catalog refresh and Hawk activation are pending an authenticated AdminOperator
refresh. The existing product device credential cannot authorize that endpoint;
its request returned HTTP 401. No installation, session rebind, Controller
restart or Machine restart has been performed by this release.

## Release identity

| Field                               | Value                                                                     |
| ----------------------------------- | ------------------------------------------------------------------------- |
| Provider                            | Codex 3.1.18                                                              |
| Previous installed Provider         | Codex 3.1.16                                                              |
| Composite SHA-256                   | `49f66c510b53b3839abb18cb6ebd5855ec856526f5fbfedb9895faee8389fcf2`        |
| Package SHA-256                     | `9e5d18d08f1e1e2af5980e5c6de9cae570c102683631bb4d6cdb6ed2025d2d5f`        |
| Publisher                           | `cowboy-first-party`                                                      |
| Publisher public key fingerprint    | `SHA256:a/VJzmHD/94vMVQMNZTktSR9P3apkkKnnWzXrtn02hg`                      |
| Public contract fingerprint         | `sha256:8a0e0b4844f07115927bacf98b88e0ac7c2081535472f8174eb281bc0c4a9d19` |
| Authentication contract fingerprint | `sha256:7f8d14c66a1796cc59514c5ac3b9e41beac856b3a13af8851447b52afa7eeb70` |
| Native CLI                          | 0.153.4, unchanged                                                        |
| ACP source baseline                 | 1.10.0, `061f9a4a2e463a220d7a3ab2ae5e9732837085ef`                        |
| Targets                             | Linux x86_64, macOS aarch64                                               |
| Published at                        | `2026-09-12T08:15:27.205Z`                                                |

The Provider owns a source patch and launcher; exact source, patch, launcher and
bundle hashes are included in each adapter archive. Rebuilding after integration
with the latest remote main produced identical runtime and signed release bytes.
No shared runtime component or other Provider version changed.

## Acceptance

`nix develop -c just check` passed on the integrated commit, including Plugin
and Provider gates, 946 Rust library tests, 1,369 Web tests, the isolated
PostgreSQL checks, lint, type checks, feature builds and release builds. The ACP
source builder passed its full suite: 550 passed, 26 skipped, plus type checking
and bundling.

The actual Linux worker passed initialization/session-new without inference,
stop/descendant drain and distinct-generation coexistence against Codex 3.1.16.
The released CLI and adapter passed credential-free probes on an actual arm64
Mac. These are private-home fixtures; no Provider Service login was performed.

| Copied native rollout                                  | Cold projection restore | Warm restore | ACP bytes received |
| ------------------------------------------------------ | ----------------------: | -----------: | -----------------: |
| Original, 152 generated images, 220,926,098 bytes      |                 1.962 s |      0.542 s |              9,932 |
| Synthetic expansion, 1,000 images, 1,312,904,160 bytes |                10.278 s |      2.666 s |              9,932 |

Both fixtures used the actual packaged entrypoint with all three leading `-c`
options. Requests preserved native identity, used `excludeTurns: true`, replayed
no display history and created no new thread or model turn. The original rollout
SHA-256 was `75960f3ba9c1d8abb804c623f9a472e1f268aad0d7e7256544421901123a21bb`.
The large fixture demonstrates bounded resume output, not constant native
projection cost or a universal latency guarantee.

The exact signed release and complete public Catalog passed the running
Controller and its preceding transaction's reader:

- Current:
  `/nix/store/sfkgvwlrp5anyx6gvzgyg9f6ap6gd1nh-cowboy-controller-release`.
- Predecessor:
  `/nix/store/2dj6d6qg4c9dyiw100r6dxc2208szq71-cowboy-controller-release`.

The active profile, successful receipt, Git deployment pin and absence of an
incomplete journal agreed at publication. The installed Columbus activator
`f0d1093` snapshots the active profile for each ordinary transaction; its
retained `accepted-recovery` GC root is not an activation profile or an
automatic rollback target. No Controller transaction was needed for this
Provider release.

All five published package/runtime URLs returned HTTP 200 with matching SHA-256
and `public, max-age=31536000, immutable` cache policy. Publication
independently verified the signature before writing Catalog bytes. The immutable
publication receipt is:

`/var/lib/cowboy/plugin-catalog/receipts/codex-3.1.18-49f66c510b53b3839abb18cb6ebd5855ec856526f5fbfedb9895faee8389fcf2.json`

Detailed private receipts and gate logs are under `/tmp/cowboy-resume-diag/`.
They contain no Provider credentials or inference requests. Catalog files being
published is distinct from the running Service advertising the new version or a
Machine activating it.

## Activation remaining

After the authorized Admin refresh, verify `/api/plugins` advertises the exact
ready release. Install that exact version/digest on Hawk through the normal
Plugin operation. Use the existing compatible Provider reload plan for the
failed session, preserving its native thread, authentication runtime generation,
worktree, saved options and queued prompt. Observe native restore readiness and
read back the resulting session identity. Do not restart unrelated workers.

The [architecture decision](../codex-durable-resume.md) records the corrected
root cause, why stripping `result` is unsafe, and the separate native media
reference format/migration acceptance. This release does not emit a new native
storage format or add an automatic new-session policy.
