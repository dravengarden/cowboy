# Core dependency update — 2026-10-11

Compatible updates of the Rust and Web dependency graphs, taken with the
[Agent runtime upgrade](agent-runtimes-2026-10-11.md) of the same day. The
build toolchains do not change. Component activation is recorded at the end;
a source version alone is not evidence of it.

## Updated

| Area | Change |
| --- | --- |
| Cargo | 39 packages to their newest versions compatible with the SDKs' 1.97.1 floor, among them tokio 1.53.2, hyper 1.12.0, uuid 1.28.0, toml 1.1.8, libc 0.2.190, zerocopy 0.8.62 and brotli 9.0.0 |
| OpenTelemetry (Web) | API logs, OTLP transformer and logs SDK 0.222.0 → 0.223.0; core, resources, metrics and trace SDK 2.11.0 → 2.12.0 |
| Vite | 8.3.1 → 8.3.4 |
| Vite React plugin | 6.1.1 → 6.1.2 |
| Oxlint | 1.86.0 → 1.87.0 |
| Prettier | 3.9.9 → 3.9.10 |
| react-syntax-highlighter | 16.1.1 → 16.1.2 |
| `@types/node` | 24.19.0 → 24.19.2 |

`depsHash` and the Cargo vendor hash in `flake.nix` follow the two lockfiles.
The Zed adapter's own lockfile and the native shell's are untouched: both are
separate workspaces, and a change to the first is a Zed Plugin source change.

## Retained

- **Rust 1.99.0.** The build stays on 1.98.1. Clippy 1.99 adds
  `assert_is_empty`, which fires on about 315 `assert!(x.is_empty())` sites in
  93 files of this repository and on 7 in the Zed adapter, and the release
  deprecates `Atomic*::fetch_update` (two core call sites, one in the adapter).
  Taking it needs a lint decision (rewrite the assertions or allow the lint)
  and a Zed adapter change with its Plugin version. Measured on the candidate
  toolchain; nothing of it is in this change.
- **Bun 1.4.3 and Node 24.21.0** are the current release and LTS.
- **CodeMirror, Lezer, Vim and the editor wrapper** keep their locked versions.
  Their coupled iOS caret, IME, paste and render matrix in `web/AGENTS.md` is
  the acceptance for moving them.
- **MUI 7.3.11, KaTeX 0.16 and Mermaid 11** stay on their major lines; MUI 9,
  KaTeX 0.19 and Mermaid 12 are migrations, not compatible updates.
- **dprint 0.54.0** is an exact formatter pin; a newer one reformats sources.
- **Zed.** Upstream is at 1.23.2. The private server still carries its patch
  series against the pinned revision, as recorded on 2026-09-30.
- **Native shell.** Tauri 2.12.3, its plugins and swift-rs 1.0.9 are newer than
  the pins in `apps/native-shell/toolchain.json`. They need Xcode and device
  builds this change did not run.

## Verification

`just check-compact` passes in one run on this source: formatting, lint,
dependency policy, the Provider and composition gates, 2,074 core Rust tests,
682 Machine tests, 52 code-adapter tests, 126 private Zed adapter tests, 2,181
frontend tests, the disposable PostgreSQL checks and the release builds.

`tools/remote_impact.py` selects five remote suites for a `Cargo.lock` change.
All ran on binaries built from this source with scripted loopback APIs, no
production credential and no model request.

| Suite | Result |
| --- | --- |
| `execution-rust-tests` | 848 tests pass |
| `claude-worker` | Accepted, full scope, 92 checks |
| `codex-worker` | Accepted, 34 checks, Provider CLI 0.160.1 with executor 0.159.3 |
| `session` | Accepted, 9 checks |
| `keeper` | Accepted, 12 checks |
