# Account usage Machine placement — 2026-10-01

Source `0edfed9b360cbeac1cd240bb31d7c8ffd4aac24a` added durable account usage
placement, the Settings → About card selector and the local Operator CLI.
See [the operating contract](../usage-execution.md) and
[sanitized deployment evidence](usage-execution-2026-10-01.json).

## Activated configuration

| Account | Pinned Machine | Fresh query result |
| --- | --- | --- |
| Anthropic / Claude | `ovh` | Machine host response timed out after 45 seconds |
| OpenAI / Codex | `hawk` | Available, no refresh error |
| xAI / Grok | `hawk` | Available, no refresh error |
| DeepSeek | `hawk` | Available, no refresh error |

The matching release CLI set all four through the authorized local Operator
socket. Readback confirmed configured and eligible Machine IDs. Controller
dispatch logs independently confirmed all four targets. Automatic had happened
to select those same Machines before the pins; this release makes the selection
explicit and durable rather than dependent on future inventory ordering.

To restore these settings through a matching Controller CLI:

```sh
cowboy operator usage-executor --provider anthropic --machine ovh
cowboy operator usage-executor --provider openai --machine hawk
cowboy operator usage-executor --provider xai --machine hawk
cowboy operator usage-executor --provider deepseek --machine hawk
```

## Verification and deployment

`nix develop -c just check-compact` passed: 1,683 main Rust tests, independent
Machine/adapter slices, 1,967 Web tests, isolated PostgreSQL contracts, Clippy,
typechecking and release builds. The new placement contract passed on SQLite
and PostgreSQL. Initial gate failures exposed the task's Cargo cache-path
assumption and missing registration of the new migration checksums; both were
corrected before the successful complete gate. No published migration changed.
Native Codex diff review found no actionable defects after deterministic gates.

Controller and Web activation receipts both report succeeded/committed and
published. Public health is `ok`; version is
`22284acf30b8695b39932f6b7383bf8d`, PWA worker `cowboy-v1783`. Both the shell
and worker return `Cache-Control: no-store`. The public lazy App bundle hash
matches the immutable release and contains the selector. An unauthenticated
settings PUT returned HTTP 401. Physical iPhone selector interaction remains
unverified; the browser must reload to acquire the new PWA code.

Only Controller and Web were activated. OVH Machine PID 56151 retained its
2026-09-30 11:16:10 UTC activation time, and all eight ACP worker PIDs remained
unchanged. No Machine, Plugin, credential, SSH or network policy was updated.

## Anthropic limitation retained

The pre-release Controller already reported `Machine Plugin host request timed
out` for Anthropic. After activation and explicit pinning, dispatch to `ovh`
occurred at 09:31:21.629624 UTC; the same timeout occurred at 09:32:06.630839 UTC.
The public session overlay renders this as session-only usage and the generic
plan-unavailable message. Successful configuration is not successful quota
retrieval, and the failure does not establish an expired login or exhausted plan.

During the request, a read-only process sample found no new collector child and
an open descriptor on Claude's 107,536,804-byte runtime archive. The deployed
Machine source verifies retained runtime integrity before executing a host
operation. This is a diagnostic lead for admission/integrity-check latency,
not proof of the complete root cause. Do not bypass signature/integrity checks,
copy credentials, or restart active workers to conceal the failure. The exact
Machine binary remains `/nix/store/ymp3labfcf2rc4nqna8allljmwx6lm9h-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`.
