# Agent runtime upgrade and component release 3.49.0 — 2026-10-11

This batch takes every Agent Provider's upstream runtime to the newest version
its adapter supports and cuts component release 3.49.0 (provider-runtime
1.1.14). Publication and installation receipts are recorded at the end; a source
version alone is not evidence of either.

| Plugin          | Previous | Source  | Private dependency changes                                    |
| --------------- | -------- | ------- | ------------------------------------------------------------- |
| claude-code     | 3.19.19  | 3.19.20 | CLI 2.1.287 → 2.1.296, ACP 0.84.0 → 0.89.1                    |
| claude-deepseek | 3.1.33   | 3.1.34  | Same Claude pins; gateway unchanged                           |
| codex           | 3.4.3    | 3.4.4   | CLI 0.159.3 → 0.160.1, ACP 2.1.0 → 2.2.2, rebased owned patch |
| codex-deepseek  | 3.1.32   | 3.1.33  | Same Codex pins; gateway unchanged                            |
| gemini          | 3.1.32   | 3.1.33  | CLI 0.62.0 → 0.63.0                                           |
| grok            | 3.1.34   | 3.1.35  | CLI 1.0.46 → 1.0.50                                           |

Registry archives and integrity values were read from the npm registry on
2026-10-11 and verified before mutation. Authentication contracts, model presets
and new-session defaults are unchanged. Zed and GitHub are unaffected.

## Retained versions

- **Codex 0.161.0 to 0.162.1.** Codex ACP 2.2.2 declares
  `@openai/codex ^0.160.1`; its own lock resolves 0.160.1. The newer native
  releases are outside that range and wait for an ACP release that declares
  them.
- **Executor 0.159.3.** `components/execution-runtime/lock.json` is owned by the
  Machine release and is not part of a Plugin. Both execution Providers keep
  accepting that digest. The Codex acceptance below runs the Provider's 0.160.1
  app-server against the 0.159.3 exec-server, which is the pair a bound session
  uses after this release.
- **DeepSeek gateways.** Both stay at Columbus
  `27352e3400e5b55c6c88d6e7ac80f788e12dfb53`; their source directories have no
  later change on the freshly fetched default branch.
- **Node 24.21.0 and Bun 1.4.3** are the current LTS and release.

## Codex ACP 2.2.2

ACP 2.2 adds an app-server supervisor and pre-session hook trust. The owned
patch is rebased onto both; `plugins/codex/runtime/README.md` states the new
reader and process lifecycle. The patch applies to the pinned revision at fuzz
0, and the builder's type check and all 1,504 enabled upstream tests pass with
it (36 skipped upstream).

## Claude Code 2.1.296

Official sources read on 2026-10-11: the
[tools reference](https://code.claude.com/docs/en/tools-reference), the
[Mods events](https://code.claude.com/docs/en/plugins/mods/events) and
[reference](https://code.claude.com/docs/en/plugins/mods/reference) pages, and
the
[changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)
for 2.1.288 to 2.1.296. Every native probe was run on the exact candidate binary
(`sha256:24972e3bc859fab2b46ed4c1e51f7d6130f06d3bd550811a114640de3370d0de`) and
compared with its 2.1.287 baseline.

The default tool inventory has the same 28 tools. Hooks, permissions, Mods,
lifecycle and shell receipts equal their baselines. The differences and their
dispositions:

| Surface                                  | 2.1.287                         | 2.1.296                                                  | Disposition                                                                                                                                    |
| ---------------------------------------- | ------------------------------- | -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `Read` input                             | —                               | `allow_large`                                            | Accepted. The target's limits bound one call, so a refusal for size adds native's `allow_large is not available where this read runs` sentence |
| `Agent` input                            | —                               | `effort`                                                 | Native runs agents; passed through                                                                                                             |
| `WebFetch` input                         | —                               | `offset`                                                 | Runs where the session runs; passed through                                                                                                    |
| `TaskStop` description                   | names `name@team`               | names the agent ID                                       | Target tools keep native's description; baseline updated                                                                                       |
| `Edit` of a file that is not valid UTF-8 | Rewrote it as UTF-8 with U+FFFD | Refused, nothing written                                 | Followed, with native's text                                                                                                                   |
| `Write` of U+FFFD over such a file       | Written                         | Refused, nothing written                                 | Followed; new shared case `write_latin1_replacement`                                                                                           |
| `Edit` result of a UTF-16 file           | Ends with the file-state note   | Leaves the note out                                      | Field set as natively; text is a stated difference (below)                                                                                     |
| `Read` binary and `pages` refusals       | Old wording                     | New wording                                              | Followed                                                                                                                                       |
| Nested `CLAUDE.md`                       | Attached on `Read`              | Attached on `Read`, `Write` and `Edit`                   | Followed; the context probe gained an `Edit` step                                                                                              |
| Environment block                        | Ends at OS version              | Adds the untrusted-downloads line                        | Followed                                                                                                                                       |
| Skill listing                            | `- <directory>: …`              | `- <directory> (<name>): …` when the names differ        | Projection keeps the parenthesis                                                                                                               |
| A Mod's denial after the tool ran        | Shown as given                  | Prefixed `<Tool> ran, and a plugin withheld its result:` | The module's own failure (a target skill's command, a target image read) is stored as the failure text alone                                   |

One difference is stated instead of fixed. Native renders a Mods-answered `Edit`
result without reading `contentNotInModelContext`, on 2.1.287 and on 2.1.296, so
the text of an unread or UTF-16 file's edit still ends with the note that the
file state is current. The audit lists it with the other stated file
differences.

A handler native cannot analyse disables the whole module, and the launcher then
refuses to start instead of running locally. A draft of this change hit that (a
`tool.call` handler calling a function declared after it); the packaged
acceptance stopped at startup with
`Claude execution module is
missing or disabled`.

## Verification

Remote suites selected by `tools/remote_impact.py --base c8fa29e3`: native sets
`claude` and `codex` changed. All ran on the packaged candidate with scripted
loopback APIs, no production credential and no model request.

| Suite                                     | Result                                                                             |
| ----------------------------------------- | ---------------------------------------------------------------------------------- |
| `claude-remote-check`                     | 300 source tests pass                                                              |
| `claude-worker`                           | Accepted, full scope: 12 phases, 92 checks, 316 scripted requests                  |
| `claude-managed`                          | Accepted, 7 checks                                                                 |
| `claude-task-stop`                        | Accepted, 4 process topologies                                                     |
| `codex-worker`                            | Accepted, 34 checks, Provider CLI 0.160.1 with executor 0.159.3                    |
| `codex-turn`, `child-stop`, `codex-hooks` | Accepted on the 0.160.1 binary                                                     |
| `session`                                 | Accepted, 9 checks, with the Controller, Machine and worker built from this source |

The Codex worker harness took one binary for both the Provider's app-server and
the target's exec-server. It now accepts `provider_cli` and `provider_sha256`
and records the Provider CLI's digest, so the acceptance covers the pair this
release produces. The Codex hooks probe takes `--version` for the same reason.

`just check-compact` passed in one run on the source merged with `13e42bb4`.
On the source merged with `32b4200f` every step passed, in two runs: formatting,
lint, dependency policy, the Provider and composition gates in the first, and
from `test` on in the second, after one timing test failed under host load (see
the last section). That is 2,074 core Rust tests, 682 Machine tests, 52
code-adapter tests, 126 private Zed adapter tests, 2,181 frontend tests, the
disposable PostgreSQL checks and the release builds. The three Web-only commits
merged after it passed the component graph check and the Web type check, lint
and tests.

## Published releases

Built from clean source `2a849aa07a65f09e4ee36cc41fe12d83deb2d71b`, which is
remote `main`, signed with `cowboy-first-party-v1`, verified with its public
key and published to `/var/lib/cowboy/plugins/catalog`. The Catalog refresh
returned HTTP 200 and advertises each exact version as `ready` under component
release 3.49.0.

| Plugin | Version | Composite SHA-256 |
| --- | --- | --- |
| claude-code | 3.19.20 | `9684c88f2d97fc8bad469112ef77f9bd76f441c06793cefba995ad50d57641c0` |
| claude-deepseek | 3.1.34 | `8df4a26d4cb331c25225d084a015f010152dbd52269a45665536243bad62b7fa` |
| codex | 3.4.4 | `3b64f08e5de8caff514df78d4c0fe25f3e235b086a1d469d4afa3582fc61d7c3` |
| codex-deepseek | 3.1.33 | `15f719f4086bd26b9b92fc83cd70ff6d7c429c5776c427b806ea31fe0d38016e` |
| gemini | 3.1.33 | `eb82dbcfd9f2619164315e051006d8decba0bec5c01a9cf661f79e2f6b9d5512` |
| grok | 3.1.35 | `7ec283994bb668ed13bfd6902b750b609d0004405d34cd72694180f3cba6cc5b` |

- The Claude and Codex runtime archives of this build unpack to the same bytes
  as the candidates the remote suites accepted. The suites ran on source
  `a16f2d64`; the merges since changed the component release number, the Web
  application and one Controller module, and no runtime source.
- Each release passed Linux worker initialization, session creation, stop with
  descendant drain and coexistence with its previous release, on the
  production worker (`sha256:37cc05f4…d96a0`). The two DeepSeek releases ran
  with both sidecars.
- Each release's macOS arm64 artifacts ran on macbook-air: every CLI, adapter
  and gateway probe passed.
- The active Controller and the cold one in Hawk's system closure each read
  the Catalog with the six releases twice, on a private copy before
  publication and from the live files after it.
- All 26 published package and runtime HTTPS URLs returned bytes matching the
  SHA-256 digest in their path, through the public hostname.
- `just provider-release-coverage` passed against the live Catalog.

## Installed

`cowboy operator converge --apply` ran Hawk first, then OVH, then Falcon and
macbook-air. All 18 upgrades returned an applied receipt, nothing was skipped,
and each re-read inventory reports the exact digests above as `active` with
the previous generation retained for rollback. The final dry run is empty on
all four Machines.

| Machine | Upgraded |
| --- | --- |
| hawk | claude-deepseek, codex, codex-deepseek, gemini, grok |
| ovh | claude-code, codex, grok |
| falcon | claude-deepseek, codex, codex-deepseek, gemini, grok |
| macbook-air | claude-deepseek, codex, codex-deepseek, gemini, grok |

Each operation is `<machine>-<plugin>-<version>-converge`. No session was
stopped or restarted by this task: a running session keeps its own generation
and a new session starts on the installed one. After the OVH upgrade,
`cowboy operator usage --refresh anthropic` returned the subscription's plan
usage through the 3.19.20 collector. Local and public `/healthz` return 200.

## Not released and not checked

- No Web, Controller or Machine component was built or activated. The
  Controller's embedded fallback Catalog still names the previous versions;
  the signed Catalog supersedes it.
- The executor stays 0.159.3 and no `cowboy-workers` pin moved.
- No model request was made on the new releases by this task. The acceptance
  uses scripted loopback APIs; real subscription inference, cross-host latency
  and the other `not_checked` entries of the receipts are unverified.
- The requested independent Codex review was not run: Agent calls are off for
  the session that did this work and the request stayed unapproved.
- One unrelated test,
  `machine_plugins::telemetry::tests::deadline_stops_retry_for_every_otlp_signal_without_cancelling_the_first_attempt`,
  timed out twice inside the complete gate while the host load average was 15
  to 19, and passed alone five times and in the rerun at lower load. Its
  two-second wait was not load tolerant; `aef3d926` on `main` has since
  bounded that wait instead.

Evidence is retained in `/home/draven/tmp/cowboy-upgrades-20261010` on Hawk:
probe receipts, conformance inputs and receipts per candidate, the release
build, Linux and macOS receipts, publication, Catalog readers and each
Machine's inventory before and after.
