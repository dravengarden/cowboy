# Agent runtime and Recommended refresh — 2026-10-01

Released from clean source `364411efaa0cd7b5473aa968706f15dfc3a53821`,
including implementation commit `6c0f5365`. Both are published on remote main.
All six signed Plugins are available in the live Catalog. Hawk upgraded its
five installed Agents; ordinary Claude Code was not installed before this
task and remains available for installation rather than newly installed.

| Plugin | Previous | Released | Private dependencies |
| --- | --- | --- | --- |
| claude-code | 3.1.36 | 3.1.37 | Claude CLI 2.1.285 → 2.1.286; ACP 0.84.0 unchanged |
| codex | 3.1.31 | 3.1.32 | CLI 0.159.2 → 0.159.3; ACP 2.0.1 → 2.1.0 |
| claude-deepseek | 3.1.26 | 3.1.27 | Same Claude upgrade; gateway 0.1.1 unchanged |
| codex-deepseek | 3.1.26 | 3.1.27 | Same Codex upgrade; gateway 0.3.0 unchanged |
| gemini | 3.1.26 | 3.1.27 | CLI 0.62.0 unchanged; shared runtime closure |
| grok | 3.1.27 | 3.1.28 | CLI 1.0.44 → 1.0.46 |

## Recommended configuration

- Codex: Sol 6.1 Medium is the new-session default; Sol 6.1 Max replaces
  Sol 6 Max. Astra Medium/Max and Luna 6 Max remain.
- Claude Code: Sonnet Medium now describes Sonnet 5.5; the `sonnet` alias
  remains. Existing Opus/Fable presets are unchanged.
- DeepSeek V4.1 Flash Max, Grok High and Gemini configuration remain current.

These are versioned `plugins/<id>/provider.json` settings, with no Web model
list change. Existing sessions retain their selected model and runtime.
The repository-owned release skill remains the canonical update workflow.

Runtime component 1.1.9 and registry 3.33.0 pin this closure. App-shell 1.1.17
also records the previously shipped `532119dd` keyboard-footer change whose
component digest had not been advanced; no new app-shell behavior was added.

## Verification

- `nix develop -c just check-compact`: passed, including Provider lock/package
  checks, Rust, PostgreSQL, frontend tests, formatting, lint and release builds.
- Codex ACP 2.1.0: owned patch applies without fuzz; typecheck/build and
  1,034 upstream tests passed (33 upstream skips).
- All six exact signed releases passed Linux x86_64 worker initialize/new
  session, stop/descendant drain, and distinct-generation coexistence checks
  against the installed predecessor. Real macOS arm64 executable probes passed.
- Synthetic large Unicode Codex history passed cold/warm native resume without
  history replay, new thread or new turn.
- All five Codex presets and the changed Claude Sonnet preset passed actual
  worker configuration selection using isolated fake authentication and no
  prompt. Account entitlement was not tested. Unchanged Claude 1M aliases are
  unavailable in that hermetic account fixture; their selection is not claimed.
- Active, next-transaction recovery, cold-bootstrap and candidate Catalog
  readers passed two reads against both real Catalog copies (16 checks).
- All six signatures verified with `cowboy-first-party-v1`; both Catalogs
  received the signed packages and runtime artifacts. Refresh returned HTTP 200
  and all six exact identities were observed as `ready`.
- All 24 unique package/runtime HTTPS URLs matched their SHA-256 digests
  using the public hostname/TLS authority with explicit loopback resolution.
  This checks the origin, not an external network path. Release coverage passed.

## Hawk activation

Each operation below returned HTTP 204, a durable `completed`/`applied`
receipt, and an independently re-read active inventory with the exact digest.
The convergence report has no skips or remaining work. Authentication
generations remain unchanged (including absent generations for unauthenticated
Providers). Controller PID 3844171 and Machine PID 3852458 did not change.
No core component activation, daemon restart or existing-session recycle ran.

| Plugin | Operation ID | Installation revision |
| --- | --- | --- |
| codex | `hawk-codex-3-1-32-converge` | `installation-3f0f0320e43d319b6825b033efee5b7846047f761ea15e9f399c934043869695` |
| claude-deepseek | `hawk-claude-deepseek-3-1-27-converge` | `installation-12d82193f4714c7684b71606cbb0aec7166ecd90e77d93e7cf75eab0976b4f9e` |
| codex-deepseek | `hawk-codex-deepseek-3-1-27-converge` | `installation-061e439e29946bae4ed4740a965750a0e08c114764fc97ab9b2a8f64c6e10911` |
| gemini | `hawk-gemini-3-1-27-converge` | `installation-4e90bfa7f0d4f95ed9c729369e089bbbc7c1a8917bdbbb3dc77e73563d895315` |
| grok | `hawk-grok-3-1-28-converge` | `installation-1e12c1fe22488c74ac45ce1423890928e4e848a7e95c61ba05287d3d18c767e3` |

## Immutable identities

| Plugin | Composite artifact digest | Contract fingerprint | Auth contract fingerprint |
| --- | --- | --- | --- |
| claude-code | `sha256:5021336f79597be17d4a3fdca88fb3d93d8d41d53e4e3b8b3084ec41831938df` | `sha256:c7b6e4d2c549a8ac492536c6f51c808eef90915e469f0b244261fb17c3a4dfeb` | `sha256:ceb8d6e09bd5d375f085435d23a52c63976cdf165f07a8b6946484207af4ede7` |
| codex | `sha256:45f2e0158e59827b3c16ba60811e7bba93883e250acff08323afdb4a77722a46` | `sha256:4a95a22f75c0afc709e486074622f0883e9ec1efe826bcb5e7b36210352708f1` | `sha256:7f8d14c66a1796cc59514c5ac3b9e41beac856b3a13af8851447b52afa7eeb70` |
| claude-deepseek | `sha256:5cda0639e1870845eb4175b7a1e0b94ea54567672055af6070845a81933ac997` | `sha256:780360292f90f98e1de5fdab10b3ed0b4d201e1967d8ef6161920393f536cd95` | `sha256:2ffe2f697ae91a568eae585cc3995acb4a4080f08431004da88aaf6058bc5d17` |
| codex-deepseek | `sha256:d76ad6100e9c97c6414c7af9859c3e499f19e0bdb91b998d51d7941751b917d5` | `sha256:c4354b0e960c2a4d42db654b03ae96b9aad02441288d079290d0bd97f6a83c56` | `sha256:a7b6784231f07487f1ef34e25e8ed571cec4246bddd0e22209049f8e1493714f` |
| gemini | `sha256:eba0fe2aa3940880421058e869d524c6ed143b3b5e686bc3ecc572fd9a0d6ebf` | `sha256:22b28c5d276b7100fb2fcf917db909fbb8a5940e137937a1623efe5a3578f2af` | `sha256:e993ef7612c5ab51929aaab0a0b8837ac74f57dc5a915e6c2ffb64d0b3b5425e` |
| grok | `sha256:07dc98393087ba25df38d5ed8922b6592d445cce9b9477ed2e7be422836f5e5e` | `sha256:000475c07f82f14cb6b1b2e05151476700b0a3884033e7eab71579d118a9c5d2` | `sha256:b150dd54708a22a816fa320d96a255e85e67ff50acc68ee06744154c950ad6ee` |

All releases declare Linux x86_64 and macOS arm64. Evidence and bounded
receipts are retained on Hawk under
`/home/draven/tmp/cowboy-agent-refresh-20261001/`; macOS temporary fixtures
were removed after their receipts were retrieved.

Upstream references: [Codex models](https://learn.chatgpt.com/docs/models),
[Claude model configuration](https://code.claude.com/docs/en/model-config),
[Codex ACP 2.1.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.1.0),
and [DeepSeek updates](https://api-docs.deepseek.com/updates/). Exact npm
archive metadata and integrity pins are retained in the runtime lock.
