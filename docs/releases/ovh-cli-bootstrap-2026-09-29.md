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
