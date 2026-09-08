# Local-first telemetry and OTel rollout — 2026-09-08

Implementation commit `9467ee354e89e49708e28f86761485b0e4c24ae6` is published on
remote main. It integrates the latest Agent settings/usage fixes without
rewriting already published Plugin identities. This record is a documentation
descendant; the initial immutable application artifacts below use that
implementation commit, with `dirty: false` source receipts. The later
Controller-only Catalog reload is recorded separately below.

## Initial production components

| Hawk component | Immutable release                                                       | Successful transaction             |
| -------------- | ----------------------------------------------------------------------- | ---------------------------------- |
| Controller     | `/nix/store/x8x8na840pi841cl3lqfvi0qw8rwh7b9-cowboy-controller-release` | `1788857084960086098-9467ee354e89` |
| Machine        | `/nix/store/21jyhgcg5hqvj06471any8r4lkbj1k6f-cowboy-machine-release`    | `1788857170992006020-9467ee354e89` |
| Web            | `/nix/store/rchqj5ywdan2jnihm87b26xbaydkwy17-cowboy-web-release`        | `1788857239481952705-9467ee354e89` |

All three machine-owned transactions report `succeeded` / `committed`, with no
incomplete transaction. The Controller and Machine PIDs are 3333032 and 3336063.
The Controller activation retained Machine PID 1944409 until the separate
explicit Machine maintenance transaction. Current busy session worker PID
1982733 survived both changes. No worker or Plugin generation was forcibly
rebound, and no Plugin installation/upgrade endpoint was called.

Hawk advertises `worker-18fedc195afdb2ea8dff`, connected and online. Falcon and
Mac also remain connected and online; their Machine releases were not changed.
Busy workers retain their current generation until a safe boundary.

The public SPA and SW match the active immutable Web bytes exactly. Both have
`Cache-Control: no-store`; `/version` is `1763999acd0fb9e1adbc0321b8e7cd2c`, and
the SW is `cowboy-v1638`. `/healthz` returns 200. Installed PWAs need a hard
reload; WebSocket reconnection alone does not replace cached JavaScript.

## Signed Plugin publication

All packages use the existing independently verified `cowboy-first-party`
publisher, shared component release 2.9.0 and SDK 1.8.0. Private runtime
dependency pins were not upgraded. Codex advances to 3.1.17 because remote main
had already published different 3.1.16 bytes.

| Plugin          | Version | Composite SHA-256                                                  |
| --------------- | ------- | ------------------------------------------------------------------ |
| Claude Code     | 3.1.16  | `e0888b1cd79f1fa6821272858fd8ffc4f9311f66fb478d09325802eaafad005c` |
| Claude DeepSeek | 3.1.16  | `bb2a70051e746fc5f6769a91d2619170caa58eda2e4bbb66c53e9ff19a262f10` |
| Codex           | 3.1.17  | `e624b1498209c75038ef79f0db020eb9ac3ddcaf19c39355448b670bbde8d1ef` |
| Codex DeepSeek  | 3.1.16  | `35b91eb87c6edd727857dc1169a9138a6bef59cc9dbfcebdb3bc56f6a0638e35` |
| Gemini          | 3.1.16  | `2c0d40da60578acdfaadfe588d8f87cf3ed1d4284ac057392345a8f34095e471` |
| Grok            | 3.1.16  | `dfea00bd4008d598cbe620ee277c65763299cfe66303cb7205673f9f9aab2e36` |
| Zed             | 1.2.4   | `3b850f191d6d9b97bbcb9ec64b59c6f227c91a8b005be5e7881f6965bea0ce75` |
| Victoria        | 1.1.0   | `e61de971b57b7a925a46cb3754270873452e883e5a52cd706343e7f450038698` |

Exact signed Agent and Zed packages passed two cold reads with the active
Controller (the next transaction's rollback predecessor), plus the candidate
reader. They were published before Controller activation; all six embedded Agent
versions passed release coverage. Actual production Host/auth policy preflight
passed with both old and new Controller binaries, without connecting to storage,
changing policy, or performing login.

Victoria's new capability was kept outside the live Catalog until the new
Controller transaction had succeeded and established the next reader floor. Its
exact signed package then passed the same three-reader check and was published.
All 26 distinct package/runtime URLs passed actual download, SHA-256,
immutable-cache, ETag and byte-length checks.

**Catalog reload completed:** the user requested a Controller update instead of
a manual Admin action. Startup reloads the signed Catalog through the same
reader used by the read-only preflight. No Admin login, credential creation or
authorization bypass is needed for this machine-owned deployment workflow.

The clean documentation descendant `1d46db8affa1f8c6d55c0f51d29b3a65969fb6f3`
built `/nix/store/z5zs85n13irchglxsr4mmvq4gc1ny7cf-cowboy-controller-release`.
It resolves to the exact same application binary as the initial release:
`/nix/store/dcfpcrkssrdw2ry79alc682hq20bnhrm-cowboy-0.1.0/bin/cowboy`. The owned
Controller-only transaction `1788864676531607367-1d46db8affa1` succeeded at
`2026-09-08T10:51:44.265039534Z`, after Victoria's publication. Its rollback
predecessor is the initial telemetry Controller, not the pre-telemetry reader.

Before activation, both actual readers accepted all eight exact published
identities and the production Host/auth policy. Victoria 1.1.0 was selected as
its Catalog default. All 148 public Catalog input/key files had identical hashes
before and after activation. The new Controller PID is 3463437; Machine PID
3336063, current worker PID 3346091 and the Web symlink stayed unchanged. Public
health, all three Machine connections, the Web version and exact SPA/SW bytes
passed again. There is no remaining manual Admin action for this requested
update.

This is restart-based acceptance, not a claimed authenticated HTTP refresh or
primary `/api/plugins` request. No Plugin was installed or upgraded and Victoria
export remains disabled. Evidence is retained under
`dist/provider-runtime-cache/catalog-reload-20260908/` in `live-readers.json`,
`before.json`, `after.json` and `verified.json`. The unsuccessful unpublished-
candidate fixture invocation is retained separately: it inserted duplicate
copies of already published packages into its temporary Catalog; no live Catalog
duplication or mutation occurred.

## Telemetry behavior and evidence

The active Controller defaults to `/tmp/cowboy-telemetry-0c78c26866378413c222`,
with directory mode 0700 and file mode 0600. Retention is eight segments of at
most 8 MiB each, with daily rotation as well. Incident/accounting durability
stays separate from this bounded diagnostic store. Victoria is not installed or
selected for export; no external telemetry destination or NixOS service policy
was enabled.

Standard protobuf OTLP logs, metrics and traces endpoints all returned 401 for
unauthenticated empty probes. Passive inspection at `2026-09-08T08:50:31.499Z`
observed 76 OTLP log records, 11 metric records, and nine legacy records in the
production file, all valid JSONL. No production trace was observed in that
snapshot. The task did not inject an authenticated production telemetry request
or claim a real prompt/turn trace acceptance. Client/runtime trace propagation,
batching, sanitization and rotation have deterministic test coverage; see
[the design](../telemetry-plugins.md).

The merged-source `nix develop -c just check-compact` gate passed: 725 main Rust
tests (eight ignored), 1190 Web tests, six isolated PostgreSQL checks, and
SDK/Plugin, native shell, feature-slice, lint, dependency and build gates. The
later publisher-identity-only correction was checked with the Victoria bundle
and targeted telemetry Plugin tests; final immutable Nix artifacts were built
from the complete committed source.

All six Agents passed exact Linux worker initialization/session creation,
old/new generation coexistence, stop and descendant drain, plus real macOS
aarch64 artifact probes. Zed passed the real-binary signed install/open/drain/
reactivate fixture. No Service credentials or inference prompts were used.
Initial Linux invocations passed a symlink rather than its resolved immutable
binary, and the first Mac invocation used Bash syntax in Fish. Those failed
invocation records remain separate; corrected runs passed. Mac disposable
artifacts were removed after saving their receipts.

Private create-only evidence is retained in
`dist/provider-runtime-cache/telemetry-20260908/`. Important receipt digests:

| Evidence                  | SHA-256                                                            |
| ------------------------- | ------------------------------------------------------------------ |
| `deployment.json`         | `7386dc7bf48bf01444bf01a3fa4f7d534da2427ce87e8db63bdd951eb6e79183` |
| `readers-agents-zed.json` | `154e10db73115974d4b9969fec29f71ef66d48f30b62237e03e15c73db5da922` |
| `readers-victoria.json`   | `279f112a49a59fce2a12ae0f773f71031f7cae9dc673e26ef9489698bd3ebd0a` |
| `public-agents-zed.json`  | `a7442f63db897907981c9a6dd38fe6df6a68ea5b0e9c4bab719798017b2d9ce7` |
| `public-victoria.json`    | `a582e6f95540ff085936b20b71b4cc455f7a963799a6551a40e361737629365d` |
