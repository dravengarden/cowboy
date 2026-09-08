# Agent settings, delivery feedback and usage — 2026-09-08

Product changes are committed and published on remote main:

- `732e1bad`: current/custom configuration summaries, acknowledged preset
  selection, Astra Medium, reliable mobile send activation, queue-delivery
  acknowledgement separation, and the shared Machine registry for usage.
- `0ea5284c`: Codex and Grok collectors parse RPC lines with native streams.
  `TextLineStream` is not a runtime global. Constructing it after spawning a
  child failed before the caller acquired the RPC handle, leaving the child
  alive until the host timeout. Both real collector entrypoints now pass a
  credential-free mock-RPC smoke test; fragmented UTF-8/CRLF tests cover parsing.

The user requested that verified product fixes automatically merge into remote
main and deploy. This standing preference is recorded in `AGENTS.md`.

## Production components

| Component | Source | Immutable release | Successful transaction |
| --- | --- | --- | --- |
| Web | `732e1bad` | `/nix/store/43f5l60brvkcd1509pd6rval42id98n3-cowboy-web-release` | `1788853890719011228-732e1bad375e` |
| Controller | `0ea5284c` | `/nix/store/lz4mh4j00b9as7b3csy00gy6hs5sh5cx-cowboy-controller-release` | `1788854789729500402-0ea5284ca6d2` |

Web serves service worker `cowboy-v1637`; the public version is
`298d9711c7852a3efe0cd8a9e291d67b`. Machine PID 1944409 was retained throughout
both Controller activations. No session was explicitly rebound to a new Plugin.

## Independent Plugin releases

| Plugin | Version | Composite SHA-256 |
| --- | --- | --- |
| Codex | 3.1.16 | `a050e94e7fd7367589eba16922d3ad489a02106c1565d9267bf79ceb95119cb4` |
| Grok | 3.1.15 | `3fc8753ccf5971e03264b314444e7a0003c8d369fb2a2c0aedbe11632a8d855a` |

Both use the existing `cowboy-first-party` publisher and unchanged private
dependency pins. Codex 3.1.15 was also published during this task before live
verification exposed the separate collector defect; its signed bytes remain
immutable. Astra Medium is included in both new Codex versions.

Hawk activated both final versions with current authentication replicas (Codex
generation 5; Grok generation 22) and retained each prior Plugin generation.
Falcon and Mac installations were not upgraded. The Service can select Hawk's
newest exact collectors without rebinding any existing session.

Final live usage acceptance confirmed OpenAI observed at
`2026-09-08T08:09:02.957Z` and xAI at `2026-09-08T08:09:56.365Z`, both with
`stale: false` and no error. A request overlapping installation timed out;
a separate xAI refresh after the shared cooldown succeeded. All three
registered Machines remained connected and online. `/healthz` returned 200;
public SPA/SW bytes matched the active Web release with `no-store` headers.

Each final package passed independent signature verification, two cold reads
with the active/transaction rollback Controller and a candidate read, actual
macOS aarch64 probes, and Linux x86_64 worker initialization, session creation,
old/new coexistence and descendant drain. All eight final published URLs passed
SHA-256 and immutable-cache checks. Catalog release coverage and candidate
Catalog-only Host configuration preflight passed before activation.

`nix develop -c just check-compact` passed after each product commit, including
1174 Web tests, Plugin/Provider checks, Rust tests, six isolated PostgreSQL
checks, lint, dependency checks and production builds. Private create-only
runtime/reader/install evidence lives under
`dist/provider-runtime-cache/agent-ux-20260908/`.

Existing sessions retain their exact Plugin generation. New Hawk Codex sessions
can use Astra Medium; an existing idle session can use Load installed Provider
to adopt its new recommendations. Physical iPhone input/IME acceptance remains
separate; PITFALLS #69 is not claimed fixed.
