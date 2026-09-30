# OVH CLI bootstrap — 2026-09-29

Status: CLI correction published; OVH Machine enrollment and real Grok session
acceptance remain blocked. The OVH VPS and its eventual Machine are permanent.

## CLI correction

Implementation source: `72653b17`, published to main. The complete pinned-shell
`just check-compact` gate and final native review passed. Review findings about
workspace IDs, history/resumption, and asynchronous preparation failures were
fixed before publication.
The new receipt is linked from `docs/INDEX.md`. A separate complete coverage
audit found 160 pre-existing unindexed Markdown pages; this task introduced no
new omission and did not expand into that unrelated documentation cleanup.

The stdio ACP bridge now supports an explicit Machine and registered workspace:

```bash
cowboy serve-acp --daemon-url https://cowboy.stormbird.xyz \
  --provider grok --machine ovh --workspace matrix
```

Use the official device authorization flow before invoking it. Remote creation
uses trusted workspace IDs, scopes history/loading to that Machine/workspace,
waits for preparation and Provider readiness, and propagates terminal failures.
It does not retry creation on another Machine. Initial configuration options
retain their wait so Zed can construct model/effort controls. Local callers keep
their existing directory behavior. See the [integration guide](../integrations/zed.md).

The clean Nix CLI build passed, including its package tests. Artifact:
`/nix/store/whdndg6r27jdrv3cg9narx4wylv12vyh-cowboy-0.1.0`.
The `bin/cowboy` SHA-256 is
`42f1131c947f4b545155df58df799e1eea3cda45d3c662f66763b686bbc82de6`.
Its help exposes both options, and its actual production `operator inspect`
request worked, returning the expected HTTP 404 for the unenrolled OVH target.
The task uses this fixed artifact; no system-wide CLI override, Controller,
resident Machine, worker, or Web activation was performed.

## Permanent Machine bootstrap

The official bootstrap at `527fba89155020ee76792722fc47b1107b3657fc` was installed
on OVH Ubuntu 26.04. Archive SHA-256:
`c42507e65282113838cca5b83ccc2733d99f15de7899462614b98a914a534e83`.
The default Service state path was retained; no shortening symlink or old
Lightsail identity was used.

Native `machine-enroll` issued a one-time ticket for `ovh` / `OVH Matrix`.
Native `register` selected Matrix, Columbus, and Suger workspace roots, but
failed on Service metadata retrieval because the Service hostname resolves
into the private overlay, which OVH has not joined. No local Cowboy Machine
identity or user service was created. A later `operator inspect --machine ovh`
returned HTTP 404. Expired ticket files were removed; no Machine was revoked
or deleted. User lingering was enabled for the eventual retained user service.

The catalog inspection observed signed Grok 3.1.24 and Zed 1.20.0. Neither was
installed on OVH. Re-read the approved catalog before installation, then use
normal installation receipts and credential replication.

The September 30 (Asia/Shanghai) readback used the official
`cowboy operator catalog` endpoint and returned HTTP 200. Its newest ready Linux
x86_64 releases were Grok 3.1.25, package digest
`sha256:dc4f7e1b868ad04471fd8905c259fdd119662776f572f9f695ad68856cfc96e8`,
and Zed 1.20.0, package digest
`sha256:d723bf69a931181b30ad82a328767cec8e6626acf17bea64d7fbf2c3e58684c9`.
This is catalog evidence only. OVH's new permanent Stormbird identity remains
quarantined from peer business traffic while its daemon repair is tested;
there is still no Cowboy Machine or installation receipt. The Falcon-specific
qualification issue does not block independent OVH preparation, but does not
authorize a temporary Controller tunnel or another transport bypass.

Two CLI authorization attempts expired without confirmation. The independent
acceptance browser context and its forwarding connection were closed. No
browser cookie or Provider credential was copied. Start a fresh authorization
request and obtain operator confirmation when resuming.

## Unaccepted boundaries

Stormbird owns the missing managed transport and recovery path. This CLI adds
no Stormbird, SSH-tunnel, or SOCKS protocol to Cowboy. Actual Grok interaction,
SSH from that session, Code surface, reconnect/replay, restart persistence,
resource peaks, fault recovery, and physical-client regression remain pending.

## September 30 activation and client authorization follow-up

The bootstrap observations above are historical. OVH is now enrolled as the
permanent Machine `ovh`, with its default Service state directory and the
Matrix, Columbus, and Suger workspaces. Official `operator inspect` returns
HTTP 200. Grok 3.1.25 and Zed 1.20.0 have completed signed installation receipts;
Grok credential replica and materialization are current at generation 1025.
The fleet-owned activation and encrypted backup evidence is recorded in
Columbus at `machines/ovh/docs/cowboy-activation-2026-09-30.json`.

Interactive acceptance is still blocked on product client authorization.
The last browser request remained pending on the server and expired without
producing a CLI credential. A user's Cardea approval alone is not evidence of
the client authorization exchange completing.

A confirmed Web defect discarded the `kind` in the Controller's HTTP 428
`session_reauthentication_required` response because that response has no
`message` field. The generic recent-auth retry could consequently select a
Passkey when the session specifically required primary login. The Web fix
preserves the closed primary/passkey kind and uses the required ceremony before
retrying the protected operation once. It does not weaken the server policy,
extend link expiry, automatically approve a newly opened link, or grant host
Operator delegation access to user sessions. Regression tests cover the actual
message-less response, both ceremony kinds, and approval continuation into the
completed UI state. Web typecheck, lint, all 1,954 tests, and build pass.
This establishes a code defect and its regression fix; it does not establish
that every symptom in the physical iPhone screenshots had the same cause.

Web revision `b14fc8f8455571fcf6fd489c171e4b34ed457fbf` was published and
activated as `/nix/store/9idiklgnk77ddnrlvh75hhsq84lyzcxz-cowboy-web-release`.
The Hawk Web activator recorded transaction
`1790772562707346069-b14fc8f84555`, outcome `succeeded`, phase `committed`,
at `2026-09-30T12:49:22.750111846Z`. Readback returned `cowboy-v1772` and
`/healthz` returned `ok`; Controller PID 603952 remained unchanged. Native diff
review found no actionable defects after the deterministic Web gates passed.

## Managed Grok session acceptance

The official CLI authorization subsequently completed and persisted its private
sender-constrained credential. `cowboy serve-acp --provider grok --machine ovh
--workspace matrix` created retained session `sess-1790767522144` through the
Controller. Grok 3.1.25 reported Grok 4.7 with high reasoning effort and streamed
the exact requested marker `OVH_GROK_READY_20260930`. These are real
Cowboy-managed session results, not a separately launched native CLI.

Initial shell calls failed because the packaged SSH client in the isolated
Provider HOME could not discover the operator's home-only aliases. Columbus
revision `9bf8e60d` moved the existing restricted client configuration into an
ordinary system SSH include, scoped to local user `ubuntu` and exact Hawk
aliases. After activation, the same session successfully executed
`ssh -o BatchMode=yes -o ConnectTimeout=10 hawk hostname` and
`ssh -6 -o BatchMode=yes -o ConnectTimeout=10 hawk id`, reporting exit zero,
`hawk`, and the non-root `matrix-agent` identity. No Provider credential or
private SSH key was copied into its runtime home. Falcon remains a recorded,
deferred failure. The packaged SSH warning about Ubuntu's GSSAPI option remains
visible and did not prevent the successful calls.

Cold loading from a newly started ACP client exposed a separate CLI race:
`initialize` could finish before the asynchronous WebSocket bootstrap populated
the session cache. Loading then falsely reported `unknown cowboy session`.
Session list/load now use the existing bounded daemon-readiness wait, leaving
initialize nonblocking and retaining Machine/workspace/Provider checks. A
regression holds both reads pending with an empty cache, then delivers bootstrap
and verifies successful load plus missing/wrong-Machine rejection. An actual
fresh candidate CLI loaded the retained production session immediately after
initialize and replayed the successful tool results. Effort changed to low
through the ACP config API; a subsequent turn recalled the original marker and
correctly distinguished the successful latest SSH checks.

One no-tool follow-up submitted at a WebSocket reconnect boundary timed out at
the acceptance driver's 300-second limit while the session reported idle.
Controller PID remained unchanged. A fresh client could load and continue the
session afterwards. Preserve this as a prompt-recovery failure requiring
diagnosis; neither cold-load repair nor successful later interaction establishes
lossless reconnect, exactly-once replay, or a network-fault acceptance pass.
Physical browser completion, Code surface, follow-up queue, context usage,
host reboot, formal fault matrix, and mobile regressions remain separate gates.

Cancellation was sent after the first streamed character of a requested
5,000-line numeric response. Output stopped after 160 numbers; the prompt
completed 52.996 seconds after cancellation with `end_turn`, not `cancelled`.
A following prompt returned `OVH_CANCEL_FOLLOWUP_OK`. Continued usability is
verified, but cancellation latency and stop-reason fidelity remain findings;
this is not a full cancellation acceptance pass.

The complete gate initially encountered Deno's executable permission check on
the task's relocated Cargo cache: `target` is a symlink to the storage volume.
The conformance recipe now grants run permission to the canonical executable
path and invokes that same path, retaining the single-executable allowlist.

The final integrated `just check-compact` gate and native diff review passed.
Published revision `ca59271466d7ee6b68813672a6f283d337d5dbca` produced the clean
Nix artifact `/nix/store/dgawpz267mbjmdr398ml7a6p177ljzxh-cowboy-controller-release`;
its `bin/cowboy` SHA-256 is
`400d8482870c2f10abd41be46fc3e41f3b56645909c0432739137d428a2a5f19`.
Only that artifact's CLI was adopted for acceptance; the Controller release
profile was not switched. Controller PID 603952 and OVH Machine PID 56151
remained unchanged, and the Machine reported zero restarts.

The exact immutable CLI, with no pre-list polling or artificial startup delay,
loaded the retained session immediately after `initialize`, replayed its
history, restored reasoning effort to high, and streamed
`OVH_COWBOY_RELEASE_READY` with `end_turn`. The permanent Machine and Grok
session remain retained. The CLI artifact has a GC root in the protected
operator evidence directory; temporary test scripts were removed after the
receipt was recorded. Product client authorization remains available for the
remaining acceptance work and must be revoked when that work is finished.
