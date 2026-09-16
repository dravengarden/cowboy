# Content-bound Code observations — 2026-09-16

**Controller/Web published and active; Machine/Zed remain candidates.** The
[closed content-bound read](../plugin-content-bound-reads.md) supports language,
symbols and hover only when the retained native text equals the complete
displayed text. Mismatch never reloads, reopens or replaces an owner. An
in-flight edit/undo ABA invalidates the result even if its bytes return to the
same content. The result is not a writer grant or proof of an atomic LSP
refresh. Ordinary Review has not switched to this API.

## Source and boundaries

Implementation `66074c644833d194f591ffab26b1d036908481b4` adds independent
closed Rust/private-adapter codecs, a shared data-only wire fixture, typed
browser capture/results, the core-only Machine support probe and failure-path
coverage. It also fixes the cropped Machine Nix source: the existing owned-read
codec was missing from that source set. Boundary checks include the shared
fixture while excluding the GPL adapter implementation from core/Machine builds.

The complete gate passed on clean `3ea6a86278d753a0875d55e390a323278221b6d7`.
Final source `5ea1b6eea44656eed3bd9003723c5213c2cd6009` merges remote
`3faf6afd`, retaining the independent legacy-record notification fix. That
integration changes only three Web files; Rust, component and Plugin sources,
locks, Nix definitions and justfile are byte-identical to the complete-gate
source. Web checks, both browser gates, native conformance and all 23 flake
checks passed again on the integrated source. Service-worker version is
`cowboy-v1697`. The enclosing follow-up only records this evidence.

Private Zed Plugin/adapter `1.5.0` → `1.6.0`; server stays `1.13.0`, with
`proto`/`clock`/`text` at `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`. No
dependency, public Plugin SDK, Machine protocol version, durable format,
database migration, production policy or authentication state changed.

## Immutable artifacts

- Controller release:
  `/nix/store/i4jccmj6dn3n0llzwgay4y66dbl05j03-cowboy-controller-release`.
- Controller executable:
  `/nix/store/vkwcinhdgbs84dbjmggillza23wsl79g-cowboy-0.1.0/bin/cowboy`, SHA-256
  `e7b42d18f720d0ee756fce9f89942f58da263ff0f8089a7564c1821ae6fb2558`.
- Web release: `/nix/store/939w4wllxly7hn9j08kbv3c8h2pddzk1-cowboy-web-release`;
  immutable assets:
  `/nix/store/c9p0ixaib0dfg28hcgx8qs2rlbaa0ycl-cowboy-web-0.1.0`.
- Static Linux x86_64 adapter candidate:
  `/nix/store/c9rmd5rx6bfw131l3zcm7cri62k477f4-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-adapter`,
  SHA-256 `58ddcff029845d8b6e00890aafe48c89ac2373d3bdde4c3728e5764aebd66bc0`.
- Unchanged server:
  `/nix/store/xjhjhq461q2qfwir9vmwbnaq7qxp3v11-cowboy-zed-server-1.13.0/bin/cowboy-zed-server`,
  SHA-256 `5829fe9d9f0b7a5a27129dc217cc9954c3b4334da5da2426bffe55423e723ae5`.
- Data package SHA-256:
  `ae4779cae46bfc53afc1c34338fdd763c09399fc46a5e16bc4373549b6721df5`.
- Unsigned bound candidate identity:
  `sha256:20ee6006b65b0fe6404b00fb59ed633eda3a3a9f7ef42e62e51a02af443fadbf`.

The package, runtime matrix and unsigned envelope are under `dist/plugins/zed`.
Their planned immutable HTTPS URLs are not evidence of upload or availability.
The final runtime receipt names `5ea1b6ee` and reproduces the accepted bytes;
both binaries pass isolated probes and ELF checks for no `INTERP`/`NEEDED`. No
Catalog publication, registered-Machine installation or worker replacement was
issued by this task.

## Accepted checks

The pinned-shell `RUST_TEST_THREADS=1 just check-compact` passed: **1,346 main
Rust, 308 standalone Machine, 26 core-adapter, 56 private Zed, 1,577 Web and 17
isolated PostgreSQL tests**, plus formatting, strict Clippy, types, features,
dependencies, Plugin/component contracts and release builds. Main/Machine suites
retain 30/two explicit ignored tests; PostgreSQL and the applicable static Zed
gate run separately. The integrated Web lint/typecheck and all 1,577 tests
passed again. Both full flake checks passed all 23 checks, including independent
cropped Machine compilation and private GNU adapter integration.

Actual isolated Firefox `151.0.1` passed eight owner/content cases and six
product-context cases. Content cases use WebCrypto, nonempty shared wire data,
StrictMode replay, displayed-text replacement, rejection of late hover replies,
mismatch without hidden reload and explicit original-owner cleanup. HTTP is
fixture-owned; these are not authenticated production or physical-device tests.
Fixture hashes are
`423292cd24033c481250baeb464f13b58fb2b3cbf40d6b33a780e861492d4a8c` and
`2f0be342a9e447fa0e97b1977cde576cd071f1c703178a510126f8b28c108eba`.

`just zed-plugin-conformance` passed against the exact static binaries above on
integrated source (8.54 seconds). Real temporary signing/install and native
opens establish two independent owners. Changed disk content produces explicit
mismatch for all three queries at still-valid positions. After uninstall, source
deletion and worktree rename, the original native content still reads through
both original owners, followed by drain and reactivation. The plaintext fixture
has no configured LSP; empty real hover is not nonempty LSP acceptance. Nonempty
results and native edit/undo ABA are covered by protocol/engine tests.

All four actual Controller roles independently accepted the same **81 ready
releases**, including six exact current Agent versions, and byte-identical
actual-Service `--check-plugin-hosts` reports. Roles were the candidate above,
active/next recovery `s3nli83h…` (`7bd390c6`), retained previous `gxxwkjfb…`
(`ba07f7f8`) and cold `cc09k6l7…` (`869c269f`), each resolved from actual
profiles, receipts or the active system closure. No configuration environment
was printed; these read-only checks created no Service state. Exact Agent
publication coverage passed. They do not validate an unpublished Zed candidate
in Catalog.

Development failures remain in the private evidence directory, including the
browser runner's outdated six-case expectation (updated to eight only for the
owner suite) and preliminary compilation/type assertions fixed before the
accepted runs. Existing Nix hardlink-limit, dependency-policy and Web chunk/lint
warnings remain visible; no host workaround or weaker product assertion was
introduced. An initial mistyped conformance recipe did not run a test and is not
counted as acceptance.

## Production activation

The installed machine-owned component activator committed both published
`5ea1b6ee` releases without recovery overrides:

- Controller transaction `1789532240548065200-5ea1b6eea446`; predecessor
  `/nix/store/s3nli83h1ipa87y2y064igz5v5wxk123-cowboy-controller-release`.
- Web transaction `1789532307825291729-5ea1b6eea446`; predecessor
  `/nix/store/rw29kwlkaii1ysyjknzm7phgvhv0b7wj-cowboy-web-release`.

Separate pre/post snapshots cover Controller **12:17:06–12:18:23 +08:00** and
Web **12:18:23–12:18:45 +08:00**; these are observation windows, not outage
durations. Controller PID changed `469574` → `935163`; Web restarted nothing.
All **14 worker** PID/start-time pairs, resident Machine and Victoria processes
were unchanged in each window. Machine is online on
`worker-3a889de3bf203a2378b8`, retaining its workspace revision/identity hash.
Machine profile/receipt, host closure/unit hashes, cold roots and failed-unit
sets were unchanged. The pre-existing `liveview-backup.service` failure remains;
the user failed-unit set was empty. This task cleared neither.

Local and public `/healthz` and `/version` passed. Index, admin, service worker
and both entry assets match immutable Web bytes exactly; HTML/SW are no-store
and hashed assets immutable. SPA version is `a06f5d439c990058ed6e006370c7d4c4`.
A PWA hard reload is still needed to replace an already-open JavaScript bundle.

## Evidence and remaining work

Private scratch evidence: `/tmp/cowboy-content-reads-2DRs7Vsr` (not a permanent
public artifact). Selected SHA-256 receipts:

| Evidence                      | SHA-256                                                            |
| ----------------------------- | ------------------------------------------------------------------ |
| Complete gate                 | `b40dbe66f03e583fa5d19d14a1db2bfdbf4f5a09d97834e554157fa4c93c72e6` |
| Integrated flake gate         | `7f9c429f7f62b6bb51d3198cb8b956175cbd862c3e6f13a507a3debc75fd97c8` |
| Integrated static native gate | `7fb217cba57f6c55f76b7831bbbc71a11a41fd90defefac78b84dd480dc6eb79` |
| Four-role Catalog audit       | `c20ccd06fc3b8abd8616b540360dc0684ca905b25818ab275ed190e0af1ca0bb` |
| Component/HTTP acceptance     | `2b2e00e58406a1cbb1f748ba36e3540b90dff07d10ca17d5f7db565b7ec08bb9` |
| Unsigned bound envelope       | `73339f0bb79dccac1208a465ebce9e2fcaafad452a2260dce71527bd8e6056cc` |

The [completion ledger](../plugin-refactor-completion.md) remains open. Required
work includes explicit disk/native synchronization with its own effect and
shared/dirty-buffer rules, owned navigation destinations, Review's consumer and
unresolved-resource UI, independent Machine/Code rollout, supported devices,
abandoned-browser/restart handling and separately authorized post-effect
recovery. Old Machines refuse the new core probe; deploying Controller/Web does
not enable production end-to-end content-bound reads or complete the Plugin
refactor.
