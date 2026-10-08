# Remote execution checks by change

Read this before running remote execution acceptance for a Plugin upgrade or a
change to the execution path. Run only the suites the change needs; suites it
does not select keep their accepted receipts.

```bash
nix develop -c python3 tools/remote_impact.py --base <accepted revision>
```

Use the commit of the last accepted release (or the revision whose receipts
are reused) as the base. The output lists:

- `native_changed`: native sets whose pins changed: `claude` and `codex`
  (`runtime.dependencies` in each Plugin's `provider.json`), `exec-server`
  (`components/execution-runtime/lock.json`) and `node`
  (`components/provider-runtime/lock.json`).
- `suites`: each selected suite with its `kind`, `command` and the reasons it
  was selected. `unit` suites are cheap source gates; `packaged` suites run the
  built release bytes with a real worker and keeper; `native` suites compare
  the pinned native CLI or executor alone.
- `unmapped`: remote files the map does not name. Every suite runs until the
  map covers them; extend `tools/remote_check_map.json` in the same change.

For `claude-worker`, set the conformance input's `"phases"` to the reported
list (`"all"`: omit the field; `[]`: the base startup and turn alone). Run each
listed native probe into a fresh receipt and diff it with its baseline (probe
receipts replace per-run ids and paths, so equal behavior compares equal).
Where it differs, decide whether the plugin must follow, update the baseline
in the same change, run `just claude-remote-check`, and add the phases the map
lists for that probe. The Claude-specific contract review is in
[Claude Remote compatibility](claude-remote-upgrades.md).

## What selects what

| Change | Suites |
| --- | --- |
| Version, display or preset fields of a Plugin manifest | none |
| Claude native pins | `claude-remote-check`, `claude-worker` (probes, then phases), `claude-task-stop` |
| Codex native pins | `codex-adapter-check`, `codex-worker`, `codex-turn`, `child-stop`, `codex-hooks` |
| Executor (`exec-server`) or Node lock | every suite built on them; `claude-worker` in full |
| A Claude runtime feature module | `claude-remote-check`, `claude-worker` with that module's phase |
| A Claude runtime core module | `claude-remote-check`, `claude-worker` in full |
| Codex launcher or adapter patch | `codex-adapter-check`, `codex-worker` |
| `codex-acp` launcher | `codex-adapter-check`, `codex-worker`, `handshake-recovery` |
| Non-inert keys of a Plugin manifest | that Plugin's unit gate and worker suite; Codex also `session`, whose fixture is built from it |
| Execution transport, keeper, protocol or wire modules | `execution-rust-tests`, both worker suites, `session`; keeper modules add `keeper` and `claude-task-stop` |
| Worker binary, `worker.rs` or `acp_bridge.rs` | `execution-rust-tests`, both worker suites, `session` |
| Keeper binary (`cowboy-execution-host`) | as the keeper modules |
| Machine control modules (`src/machine_*`) or Machine binaries | `execution-rust-tests`, `session` |
| Server execution-session API (`src/server/execution*`) | `execution-rust-tests`, `session` |
| Shared `components/memory-client` | `claude-remote-check`, both worker suites |
| A harness file under `tools/` | every suite whose entry imports it, transitively |

`just claude-remote-check` and `python3 -m unittest discover -s tools -p
remote_impact_test.py` pin the selection: the tests fail when a remote file is
missing from the map, the map names a missing file or recipe, or the Claude
phases differ from the conformance harness. Release pipeline gates outside
remote execution (`check-compact`, `agent-worker-conformance`,
`catalog-reader-conformance`, signing and publication) still run as the
release workflow requires.
