# Host Operator release and Claude upgrade — 2026-09-16

Status: the private Operator CLI is live and explicitly enabled on Hawk.
Claude Code **3.1.25** is signed, published and installed. A real Max account
returned five-hour and weekly quota through the production usage service.

## Problems resolved

1. The host agent could build and publish, but browser-only Operator
   authentication stopped installation requests at HTTP 401. The new
   [host delegation and release procedure](../agent-driven-plugin-release.md)
   uses kernel peer identity on a private Unix socket and the existing durable
   installer. It records `unix-uid:1000`, exact release identity and one operation
   ID. No browser credential, direct database edit or installation-pointer write
   was needed.
2. Installing 3.1.24 exposed a separate collector defect. The native CLI reported
   a signed-in Max account and `rate_limits_available: true`, but returned null
   limits with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`. Replacing that blanket
   switch with `DISABLE_TELEMETRY=1` and `DISABLE_ERROR_REPORTING=1` restored the
   actual quota request. Auto-update remains disabled. Version 3.1.25 carries
   this correction; CLI 2.1.272 and ACP 0.77.0 remain unchanged.
3. Main integration found two release omissions: the retained-record storage
   change lacked its component release, and a new code-buffer contract fixture
   was absent from the Nix source closure. Registry 3.9.0 records state-sync-idb
   1.8.0 without changing unaffected component inputs. The exact fixture is now
   included in the Controller build. An earlier candidate was also correctly
   rejected as stale before activation; the final artifact includes the merged
   main revision and packaging fix.

## Production receipts

| Item | Evidence |
| --- | --- |
| Operator implementation | `d477aee4ed1a8ec082d4615af1f4ef962b3a52f6` |
| Activated Controller source | `7bd390c6e27a35f92afb82299bd1c93c8eb804c3` |
| Controller transaction | `1789522265509643078-7bd390c6e27a`, succeeded/committed |
| Controller release | `/nix/store/s3nli83h1ipa87y2y064igz5v5wxk123-cowboy-controller-release` |
| Quota correction | `0886506a` on remote main |
| Plugin | `claude-code` 3.1.25 |
| Artifact | `sha256:88e7b0d1a832102e09fc1d2cc3f968a831046c357a89d7d59b1977792204d1f9` |
| Package | `sha256:af0df493f185b9bbb798e5ce0ef7355077e38546165f5fe5fd035dc084882eec` |
| Host bundle | `sha256:d7e17c720caf4985f54ecc6b7758838ed86e8832de305979aeab7ec87803dfc7` |
| Installation operation | `hawk-claude-code-3-1-25-reviewed-upgrade` |
| Installation revision | `installation-da64e05385146545681138226dce87f59342124ceaded5e5c2b8165e7d167322` |
| Service / Machine outcome | `completed` / `applied`, no reconciliation fence |

The preceding `hawk-claude-code-3-1-24-reviewed-upgrade` also completed and is
retained as a separate historical operation. Both installations used the new
CLI and the same Service/Machine coordinator. Hawk reports 3.1.25 active, auth
generation 3, and current replica/materialization state.

At **2026-09-16 01:49:58 UTC**, the production Anthropic record was `available`,
with no error and `stale: false`. Five-hour and weekly utilization were both
**4%** (96% remaining). This came from Claude's native usage control, without
an inference prompt or reading credentials in the collector. It is a point-in-time
subscriber observation, not a guarantee about later utilization.

`/healthz`, `/version`, the SPA and service worker returned HTTP 200. SPA and
service-worker bytes and their `no-store` caching behavior were preserved.
Hawk remained online with `worker-3a889de3bf203a2378b8` and the same workspace
identity digest. No Machine binary activation or forced session restart ran.

## Verification

- The complete pinned-shell gate passed: 1,339 all-feature Rust tests, 305
  standalone Machine tests, 26 core adapter tests, 35 private adapter tests,
  1,512 Web tests and 17 isolated PostgreSQL tests. After concurrent main was
  merged, the affected 29 Controller buffer tests, Web type check and all
  1,538 Web tests passed. The final Nix Controller build and its tests passed.
- Nine checks against the exact final immutable Controller and the actual
  Machine release exercised the CLI/private socket/Service/Machine path in
  disposable storage. One target query and one installation step were sent.
  Default-deny, UID attribution, TCP separation, revocation, duplicate requests
  and restart observation all passed without replay.
- The final Plugin passed `just provider-check`, including eight collector
  tests and a regression for an inherited blanket traffic switch. The real
  pinned CLI and collector also returned subscriber quota after the correction.
- Linux 3.1.24/3.1.25 worker coexistence, initialize/session creation and
  descendant drain passed. Actual macOS arm64 CLI/adapter probes passed. These
  runtime fixtures used fake credentials and sent no model prompts.
- SDK signature verification, all five public artifact hashes and immutable
  caching passed. The actual active, next-transaction recovery and cold
  Controller readers each accepted the exact 3.1.25 Catalog twice before
  publication and twice afterwards.

Private local evidence is retained at
`/home/draven/tmp/cowboy-agent-release-20260916/`. The publication receipt is
`/var/lib/cowboy/plugin-catalog/receipts/claude-code-3.1.25-88e7b0d1a832102e09fc1d2cc3f968a831046c357a89d7d59b1977792204d1f9.json`.
Temporary Mac probe files were removed. Host delegation remains enabled for
future authorized agent releases; `cowboy operator disable` revokes it.
