# Gemini retirement — 2026-10-30

Status: source retired on `main`; Controller and Web at `da27c88b`, every
Machine at `895b1a19` (worker `worker-18aa815194c30ab76edd`), and the six Agent
Plugins of component release 3.72.0 installed on all four Machines.

## Change

Gemini was sunset on 2026-10-20 (removed from every Machine, `plugins.disabled`).
`3aaef93d` and its parents remove `plugins/gemini`, its runtime package and lock
entry, and Gemini-only paths and documentation. Historical sessions, usage rows
and Catalog entries naming `gemini` still load; the `acp_session_mode_yolo_v1`
SDK option remains for stored session behavior. `plugins.disabled = ["gemini"]`
stays valid and must stay set while the Catalog holds Gemini releases.

The registry gained explicit `plugin_removals`: a removed identity must exist in
the previous release, can never return, and earlier releases stay immutable.
Release 3.72.0 records `plugin_removals: ["gemini"]`. Removing Gemini's
`provider.json` fixtures from Provider SDK tests moved provider-sdk to 3.1.17,
plugin-sdk to 1.12.4 and provider-ui to 3.1.23, so every Plugin moved one patch
version with unchanged upstream executables.

## Delivery

| Step | Result |
| --- | --- |
| `just check-compact` | Passed (branch, before merge) |
| Worker pin | `895b1a19` retains `da27c88b` (which also carries the session fork and a refreshed Cargo vendor hash) |
| Linux coexistence, new worker vs. previous releases | Passed for claude-code 3.20.21, claude-deepseek 3.1.48, codex 3.5.14, codex-deepseek 3.1.49, grok 3.1.49, deepseek-harness 1.0.6 |
| Real macOS arm64 probes | Passed for all six |
| Catalog readers | Active Controller (`da27c88b`, Provider SDK 3.1.17) reads all six; rollback, accepted-recovery and bootstrap readers skip them safely |
| Publication | Six releases ready, 26 public URLs match, coverage passed |
| Hawk / Falcon Machine | Component activators committed `895b1a19` |
| OVH Machine | `tools/ovh-machine-host-release.sh --workers`, receipt `/var/lib/columbus/ovh-machine-895b1a191d9f-20261030T111802Z` accepted; busy workers retained, idle ones handed off natively |
| macbook-air Machine | Rust 1.98.1 build at `895b1a19` and owned `--refresh`; installed hashes match; launcher only lost its Gemini fallback line; backups `*.pre-gemini-retirement-20261030` and `recovery-gemini-retirement-20261030` |
| Convergence | 17 upgrades, all HTTP 204; nothing remaining |

zed 1.20.12, github 0.2.7, matrix 1.0.4 and review 1.0.8 are versioned in
source only; their installed releases are unchanged.

A concurrent session started its own OVH maintenance eleven seconds earlier
toward the same worker generation. Its candidate was rolled back by its timer;
this task's first attempt refused before touching the host ("Host configuration
changed before maintenance"). The second attempt, five minutes after the host
was stable again, was accepted.
