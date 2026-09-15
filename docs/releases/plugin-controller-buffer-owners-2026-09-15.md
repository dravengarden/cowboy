# Controller buffer-owner candidate — 2026-09-15

Historical status at the original gate: source and immutable Controller
candidate verified; **not activated**.
The production release gate refuses six missing upstream Agent publications.
No Controller, Web, Machine, worker, Plugin installation or policy was changed
by this task. This is not an end-to-end native/Review rollout receipt.

Follow-up: the [six independent Agent publications](agent-publication-2026-09-15.md)
now close this original prerequisite. The later, independently activated
Controller `0fded719` includes the buffer-owner implementation, so the old
candidate was not deployed over it. That follow-up records the exact live
reader floor, automatic Catalog adoption and remaining authenticated/native
acceptance, plus the separate newer Claude Code `3.1.23` publication requirement.
The original failed gate and production snapshot below remain historical evidence.

## Candidate

- Implementation commit: `70aecfad`.
- Integrated clean build source: `7b90e465676a162fc8d524bbe6eb3ebbbd73c09f`.
- Controller release:
  `/nix/store/cyimldyl11r47bfcbnr5n2r318rj712p-cowboy-controller-release`.
- Executable:
  `/nix/store/var99mgnh9q33mqywrkxdrcq1pwydn0n-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `fa8fdd542ec8fec5c23220b6033acdb30f44b882596d012ed1b3066d641d4da6`.

The [additive API](../plugin-controller-buffer-owners.md) retains the original
product user, Session incarnation, Machine connection and native reference.
Core owns bounded prepare/open/query/release continuations independently of the
HTTP observer. A possible mutation cannot be replayed or expired after a missing
reply. Original-user cleanup survives Session deletion and cwd ABA without
resolving replacement paths. Closed Product-only authentication reuses the
existing original-credential recheck; separate admin-cookie precedence is not
inherited by this resource surface.

There is no Plugin/SDK, Machine protocol, native executable, durable journal,
database migration or Web-runtime change in this implementation. It consumes
the earlier [Machine/Zed 1.3.0 candidate](plugin-native-buffer-leases-2026-09-15.md),
which remains independently publishable/installable, not activated by a
Controller release. The existing Review buffer and language endpoints are
unchanged. Native language-reader lifetime, browser pending/unknown handling,
abandoned-owner cleanup, restart restoration and general recovery remain open.

## Verification

Twenty new tests cover the Controller owner/API and Product-only credential
continuation; an existing admin-precedence test also exercises the new product
capture path. The focused suites passed 19 buffer tests and 19 authority tests.
They use actual Hub observations, Machine correlation/connection handling,
deadline-owned tasks, a real loopback HTTP router and temporary SQLite
credentials. Machine/native replies and the router's explicit local principal
remain fixtures, not real production login or immutable cross-process native
generation acceptance.

The complete `nix develop -c just check-compact` gate passed from the integrated
source, including Plugin/component/package gates, dependency checks, strict lint,
type checks, isolated feature builds and shipped builds:

- Rust all-feature suite: **1,317 passed**, 29 explicitly ignored.
- Standalone Machine: **300 passed**, 2 explicitly ignored.
- Core Code adapter: **26 passed**; private Zed adapter: **25 passed**.
- Web: **1,495 passed**.
- Owned isolated PostgreSQL gate: **17 passed** separately.

The immutable Nix Controller build passed. Initial attempts found test-helper
visibility, a loopback fixture's missing Rustls initialization, and strict lint
issues; these were corrected before the accepted complete gate. No product
dependency or host toolchain was changed to fix them.

## Publication coverage blocks activation

Actual candidate, currently active Controller and cold Controller binaries all
read exactly the same **72** ready releases in `/var/lib/cowboy/plugin-catalog`.
Their closed-environment checks created no disposable Service data path. This
establishes Catalog readability, **not publication coverage**.

| Reader role | Source | Immutable Controller release |
| --- | --- | --- |
| Candidate | `7b90e465` | `/nix/store/cyimldyl11r47bfcbnr5n2r318rj712p-cowboy-controller-release` |
| Active / next ordinary transaction predecessor | `2f2c0403` | `/nix/store/hqxf3ma74h3fnb1h612h2hdnazw7ww7x-cowboy-controller-release` |
| Actual cold closure | `869c269f` | `/nix/store/cc09k6l788mhchy321ckgg0yryb1hg12-cowboy-controller-release` |

The canonical read-only `just provider-release-coverage
/var/lib/cowboy/plugin-catalog` failed for every source Agent version:

| Plugin | Missing exact signed publication |
| --- | --- |
| Claude Code | `claude-code@3.1.22` |
| Claude Code · DeepSeek | `claude-deepseek@3.1.18` |
| Codex | `codex@3.1.21` |
| Codex · DeepSeek | `codex-deepseek@3.1.18` |
| Gemini | `gemini@3.1.18` |
| Grok | `grok@3.1.19` |

These versions came from already-integrated main, not this buffer-owner change.
Do not weaken the gate, substitute older manifest versions or fabricate signed
receipts to activate the Controller. Complete each Plugin's independent release
authority, platform/runtime gates, signature, artifact publication and Catalog
verification first. Publication still does not authorize installation. Recheck
the exact current/next-recovery/cold reader floor and fresh source ancestry
immediately before a later Controller activation; this candidate cannot be
activated against a divergent newer main.

## Observed production baseline

Read-only snapshot at **21:44:14 +08:00**:

- Controller remained `2f2c0403`, PID `2793404`, successful transaction
  `1789472583452464498-2f2c0403b839`.
- Machine was independently updated to `9b960614`, PID `2990908`, online on
  `worker-1651266a4c81619b4cc6`; its release is
  `/nix/store/xl6k38q24z4rgwjcqy6c97lxk07msz4q-cowboy-machine-release`.
- Web was independently updated to `fa94df37` at
  `/nix/store/wmlvcp39jzhrvb56xp6gvadmy04ys7df-cowboy-web-release`.
- Twelve workers were present. The installed Zed slot still identified `1.2.2`
  and generation `b5695b999b6be2c396bb22c74079a8c217b064874dd04f53f1eeedb838b2e699`.
  This is installation metadata, not attestation of the running native image.

No activator was dispatched. Health, publication and installation acceptance
must not be inferred from the successful source build.

Local logs and read-only evidence are under
`/tmp/cowboy-controller-buffers-Hcx4udeI`. `reader-audit.json` deliberately records
`accepted: false` because coverage is absent; `provider-release-coverage.log`
preserves the separate canonical rejection. No production credentials appear
in these receipts.
