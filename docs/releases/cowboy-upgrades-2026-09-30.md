# Cowboy dependency upgrade — 2026-09-30

This batch updates the supported core dependency graph, build toolchains and
independently versioned Agent Plugins. Publication, component activation and
Machine installation require their own receipts; source versions alone are
not evidence of a production upgrade.

## Candidate source

| Area | Previous | Candidate |
| --- | --- | --- |
| Rust build toolchain | 1.97.1 | 1.98.1 |
| Deno | 2.9.5 | 2.9.7 |
| Core build Node | 24.15.0 | 24.21.0 LTS |
| Private Agent Node | 24.19.0 | 24.21.0 LTS |
| React / React DOM | 19.2.6 | 19.3.0 |
| TypeScript | 5.9.3 | 7.0.2 |
| Vite | 8.2.1 | 8.3.1 |
| Vite React plugin | 5.2.0 | 6.1.1 |
| Oxlint | 1.67.0 | 1.86.0 |
| Mermaid | 11.16.0 | 11.17.2 |
| Prettier | 3.9.5 | 3.9.9 |
| SQL formatter | 15.8.2 | 15.9.0 |

Cargo resolves 159 compatible package updates while preserving the SDKs'
1.97.1 minimum compiler. Rust 1.98 detects redundant trait imports and
fixed-size chunk iterators; those are corrected without changing protocol or
storage behavior. Six HTTP helpers deliberately retain their owned Axum
rejection response and document that local lint expectation. The dependency
policy permits exactly WebAuthn's base64 0.21.7 alongside the modern HTTP
stack's 0.23; advisory, license and source checks still cover both.

TypeScript 7 removes `baseUrl`; all path mappings are now explicitly relative.
The application already uses Vite 8 without Babel transforms, so the React
plugin's version-6 migration requires no compiler feature opt-in. References:
[TypeScript 7 migration](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/)
and [Vite React changelog](https://github.com/vitejs/vite-plugin-react/blob/main/packages/plugin-react/CHANGELOG.md).

## Independent Plugins

| Plugin | Previous | Candidate | Private dependency changes |
| --- | --- | --- | --- |
| Claude Code | 3.1.34 | 3.1.35 | CLI 2.1.285, ACP 0.84.0 |
| Claude DeepSeek | 3.1.24 | 3.1.25 | Same Claude pins; gateway unchanged |
| Codex | 3.1.28 | 3.1.29 | CLI 0.159.2, ACP 2.0.1, rebased owned transport patch |
| Codex DeepSeek | 3.1.24 | 3.1.25 | Same Codex pins; gateway unchanged |
| Gemini | 3.1.24 | 3.1.25 | CLI 0.62.0 |
| Grok | 3.1.25 | 3.1.26 | CLI 1.0.44 |
| Zed | 1.20.0 installed | 1.20.2 | SDK closure from 1.20.1 plus compatible iterator fix |

Shared runtime component 1.1.8 has append-only registry release 3.31.0.
Authentication contracts and model presets remain unchanged. Exact registry
archives and integrity values, including Node's published checksums, were
verified before mutation. The two private gateways remain pinned to Columbus
`27352e3400e5b55c6c88d6e7ac80f788e12dfb53`; their source directories have no
newer changes in the freshly fetched default branch.

Codex ACP 2.0.1 changes its input reader. The owned patch preserves bounded
strict UTF-8 framing, redacted failures, explicit pending-request cleanup and
configured native executable arguments. It also retains upstream fragmented
Unicode, valid final-frame and linear large-message tests. Typecheck, build
and all 1,027 enabled upstream tests pass; 33 upstream tests remain skipped.
The packaged worker, generation coexistence and cold/warm resume checks are
separate release gates.

GitHub 0.1.0's previously blocked publication is complete; see the
[accepted host-floor and publication follow-up](workspace-extensions-2026-09-29.md#publication-follow-up--2026-09-30).

## Retained versions and scope

- Zed upstream 1.21.0 rejects all seven current private server patches with
  zero fuzz. Retain the exact private upstream revision and server 1.6.0;
  accepting a newer upstream requires porting and revalidating the native
  acquisition, edit, ownership and resource bounds.
- MUI remains on 7.3.11. Its
  [v9 migration](https://mui.com/material-ui/migration/upgrade-to-v9/) removes
  component APIs and changes supported browsers, focus and DOM behavior.
  That UI migration is not accepted by this compatible dependency batch.
- CodeMirror, Lezer, Vim and the editor wrapper keep their locked versions.
  Their coupled iOS caret/IME/paste/render/expand matrix remains required by
  `web/AGENTS.md`; Firefox evidence does not establish that acceptance.
- KaTeX and Mermaid keep their current major lines and renderer integration.
  Node stays on the supported 24 LTS line, rather than the current 26 line.
- The iOS device linker blocker remains: latest swift-rs is still 1.0.8.
  No device release or physical-iPhone caret fix is claimed.

## Evidence

Task evidence is retained in `/home/draven/tmp/cowboy-upgrades-20260930`.
The initial production snapshot records 18 ACP workers and the exact component
profiles. Runtime probes use temporary homes and fake authentication, never
the Service credential. Production upgrades use the existing delegated
Operator and preserve its saved operation identities and active-session fences.

The Firefox regression run passes 16 suites / 131 checks across workbench,
Plugin management, Review and IndexedDB. Full quality, signed runtime and
production rollout receipts are recorded after their completion below.
